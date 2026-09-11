#!/usr/bin/env python3
"""Confirm an eltoo-style stale-state override on isolated Bitcoin Core regtest.

Usage: check_eltoo.py BITCOIND BITCOIN_CLI ELTOO_VECTORS
Build the exporter with `cargo build --locked -p sapio_integration_tests
--example eltoo_vectors`. No existing node, wallet, or user funds are used.
"""

from decimal import Decimal
import json
from pathlib import Path
import socket
import subprocess
import sys
import tempfile


def main():
    bitcoind, bitcoin_cli, exporter = map(lambda p: str(Path(p).resolve()), sys.argv[1:])
    initial = json.loads(subprocess.check_output([exporter], text=True, timeout=120))

    def manifest(document):
        return {key: document[key] for key in
                ("funding", "sponsor", "delay_blocks", "payouts")}

    delay = initial["delay_blocks"]
    assert delay >= 2, "the demo must distinguish the old and new contest deadlines"
    assert initial["sponsor"]["count"] == 3
    with tempfile.TemporaryDirectory(prefix="sapio-eltoo-") as directory:
        data = Path(directory)
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        with (data / "node.log").open("w") as log:
            node = subprocess.Popen(
                [bitcoind, "-regtest", "-server=1", f"-datadir={data}",
                 f"-rpcport={port}", "-listen=0", "-connect=0", "-dnsseed=0",
                 "-discover=0", "-acceptnonstdtxn=0", "-fallbackfee=0.00001",
                 "-printtoconsole=1"],
                stdout=log, stderr=subprocess.STDOUT,
            )

            def rpc(method, *arguments, wallet=False):
                command = [bitcoin_cli, "-regtest", f"-datadir={data}",
                           f"-rpcport={port}", "-rpcwait", "-rpcwaittimeout=30"]
                if wallet:
                    command.append("-rpcwallet=eltoo")
                command.append(method)
                command.extend(json.dumps(arg, separators=(",", ":"))
                               if isinstance(arg, (dict, list, bool)) else str(arg)
                               for arg in arguments)
                output = subprocess.check_output(command, text=True, timeout=40).strip()
                if not output:
                    return None
                try:
                    return json.loads(output)
                except json.JSONDecodeError:
                    return output

            try:
                rpc("getblockchaininfo")
                rpc("createwallet", "eltoo")
                miner = rpc("getnewaddress", "", "bech32m", wallet=True)
                rpc("generatetoaddress", 101, miner)

                def fund(specification):
                    sats = specification["amount_sats"]
                    amount = f"{sats // 100_000_000}.{sats % 100_000_000:08d}"
                    txid = rpc("sendtoaddress", specification["address"], amount, wallet=True)
                    raw = rpc("gettransaction", txid, wallet=True)["hex"]
                    decoded = rpc("decoderawtransaction", raw)
                    matches = [out for out in decoded["vout"]
                               if out["scriptPubKey"].get("address") == specification["address"]]
                    assert len(matches) == 1, matches
                    return f"{txid}:{matches[0]['n']}"

                funding = {
                    "funding": fund(initial["funding"]),
                    "sponsors": [fund(initial["sponsor"]) for _ in range(3)],
                }
                # The channel can stay off-chain beyond the contest delay.
                # Its contract has no unilateral settlement branch.
                rpc("generatetoaddress", delay + 1, miner)
                funding_file = data / "funding.json"
                funding_file.write_text(json.dumps(funding))
                vectors = json.loads(subprocess.check_output(
                    [exporter, str(funding_file)], text=True, timeout=120,
                ))
                assert manifest(vectors) == manifest(initial), "funding changed channel terms"
                auth = vectors["authorizations"]
                assert len(bytes.fromhex(auth["direct"])) == 64
                assert auth["direct"] == auth["rebound"], "rebinding changed the authorization"
                transactions = vectors["transactions"]

                def check(name, allowed, reason=None):
                    result = rpc("testmempoolaccept", [transactions[name]])[0]
                    assert result["allowed"] is allowed, (name, result)
                    if reason is not None:
                        assert result.get("reject-reason") == reason, (name, result)
                    print(f"{name}: {'accepted' if allowed else result['reject-reason']}", flush=True)
                    return result

                direct = rpc("decoderawtransaction", transactions["update_3_direct"])
                rebound = rpc("decoderawtransaction", transactions["update_3_rebound"])
                for field in ("version", "locktime", "vout"):
                    assert direct[field] == rebound[field], field
                assert [vin["sequence"] for vin in direct["vin"]] == [
                    vin["sequence"] for vin in rebound["vin"]]
                assert direct["vin"][0]["txid"] != rebound["vin"][0]["txid"]
                assert direct["vin"][0]["txinwitness"][0] != rebound["vin"][0]["txinwitness"][0]
                check("update_3_direct", True)
                check("update_1", True)
                old_txid = rpc("sendrawtransaction", transactions["update_1"])
                rpc("generatetoaddress", 1, miner)
                old_height = rpc("getblockcount")
                check("settlement_1", False, "non-BIP68-final")
                equal = check("equal_update_from_1", False)
                assert "Locktime requirement not satisfied" in equal["reject-reason"], equal

                check("update_3_rebound", True)
                new_txid = rpc("sendrawtransaction", transactions["update_3_rebound"])
                rpc("generatetoaddress", 1, miner)
                new_height = rpc("getblockcount")
                assert rpc("gettxout", old_txid, 0) is None
                assert rpc("gettxout", new_txid, 0) is not None
                check("settlement_1", False, "missing-inputs")
                check("settlement_3", False, "non-BIP68-final")
                older = check("older_update_from_3", False)
                assert "Locktime requirement not satisfied" in older["reject-reason"], older

                # Mempool admission checks the next block's height. At the old
                # state's deadline, the newly confirmed update still has time.
                old_deadline_tip = old_height + delay - 1
                rpc("generatetoaddress", old_deadline_tip - new_height, miner)
                check("settlement_1", False, "missing-inputs")
                check("settlement_3", False, "non-BIP68-final")
                new_deadline_tip = new_height + delay - 1
                rpc("generatetoaddress", new_deadline_tip - rpc("getblockcount"), miner)
                check("settlement_3", True)
                settlement_txid = rpc("sendrawtransaction", transactions["settlement_3"])
                rpc("generatetoaddress", 1, miner)
                assert rpc("getblockcount") == new_height + delay
                for index, payout in enumerate(vectors["payouts"]):
                    output = rpc("gettxout", settlement_txid, index)
                    assert output["confirmations"] == 1
                    assert output["scriptPubKey"]["address"] == payout["address"]
                    assert int(Decimal(str(output["value"])) * 100_000_000) == payout["amount_sats"]
                print(f"State 3 superseded state 1 and settled after its own {delay}-block "
                      "contest window. Core checked Taproot, CLTV and CSV; WASM checked "
                      "TemplateHash, IKEY and CSFS.")
            finally:
                if node.poll() is None:
                    try:
                        rpc("stop")
                    except subprocess.SubprocessError:
                        node.terminate()
                    try:
                        node.wait(timeout=30)
                    except subprocess.TimeoutExpired:
                        node.kill()
                        node.wait(timeout=5)


if __name__ == "__main__":
    main()
