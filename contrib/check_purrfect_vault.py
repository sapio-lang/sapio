#!/usr/bin/env python3
"""Confirm Purrfect Vault cancellation and delayed withdrawal on regtest.

Usage: check_purrfect_vault.py BITCOIND BITCOIN_CLI PURRFECT_VAULT_VECTORS
Build the exporter with `cargo build --locked -p sapio_integration_tests
--example purrfect_vault_vectors`. Only an isolated node and disposable funds
are used. Unmodified Bitcoin Core checks the emulated covenant's witnesses.
"""

import argparse
from decimal import Decimal
import json
from pathlib import Path
import socket
import subprocess
import tempfile


def satoshis(value):
    amount = Decimal(str(value)) * 100_000_000
    assert amount == amount.to_integral_value(), value
    return int(amount)


def manifest(document):
    return {key: document[key] for key in
            ("funding", "sponsor", "delay_blocks", "destination")}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bitcoind", type=Path)
    parser.add_argument("bitcoin_cli", type=Path)
    parser.add_argument("exporter", type=Path)
    arguments = parser.parse_args()
    bitcoind, bitcoin_cli, exporter = (
        str(path.resolve()) for path in
        (arguments.bitcoind, arguments.bitcoin_cli, arguments.exporter)
    )
    initial = json.loads(subprocess.check_output([exporter], text=True, timeout=120))
    delay = initial["delay_blocks"]
    assert delay == 20, initial
    assert initial["funding"]["amount_sats"] == 100_000, initial
    assert initial["destination"]["amount_sats"] == 100_000, initial
    assert initial["sponsor"]["amount_sats"] == 2_000, initial
    assert initial["sponsor"]["count"] == 4, initial

    with tempfile.TemporaryDirectory(prefix="sapio-purrfect-vault-") as directory:
        data = Path(directory)
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        with (data / "node.log").open("w") as log:
            node = subprocess.Popen(
                [bitcoind, "-regtest", "-server=1", "-nosettings", f"-datadir={data}",
                 f"-rpcport={port}", "-listen=0", "-connect=0", "-dnsseed=0",
                 "-discover=0", "-acceptnonstdtxn=0", "-fallbackfee=0.00001",
                 "-printtoconsole=1"],
                stdout=log, stderr=subprocess.STDOUT,
            )

            def rpc(method, *values, wallet=False):
                command = [bitcoin_cli, "-regtest", f"-datadir={data}",
                           f"-rpcport={port}", "-rpcwait", "-rpcwaittimeout=30"]
                if wallet:
                    command.append("-rpcwallet=purrfect-vault")
                command.append(method)
                command.extend(json.dumps(value, separators=(",", ":"))
                               if isinstance(value, (dict, list, bool)) else str(value)
                               for value in values)
                output = subprocess.check_output(command, text=True, timeout=40).strip()
                if not output:
                    return None
                try:
                    return json.loads(output, parse_float=Decimal)
                except json.JSONDecodeError:
                    return output

            try:
                rpc("getblockchaininfo")
                rpc("createwallet", "purrfect-vault")
                miner = rpc("getnewaddress", "", "bech32m", wallet=True)
                rpc("generatetoaddress", 101, miner)

                def fund(specification):
                    sats = specification["amount_sats"]
                    amount = f"{sats // 100_000_000}.{sats % 100_000_000:08d}"
                    txid = rpc("sendtoaddress", specification["address"], amount, wallet=True)
                    raw = rpc("gettransaction", txid, wallet=True)["hex"]
                    decoded = rpc("decoderawtransaction", raw)
                    matches = [output for output in decoded["vout"]
                               if output["scriptPubKey"].get("address") == specification["address"]]
                    assert len(matches) == 1, matches
                    assert satoshis(matches[0]["value"]) == sats, matches
                    return f"{txid}:{matches[0]['n']}"

                funding = {
                    "funding": fund(initial["funding"]),
                    "sponsors": [fund(initial["sponsor"]) for _ in range(4)],
                }
                rpc("generatetoaddress", 1, miner)
                funding_file = data / "funding.json"
                funding_file.write_text(json.dumps(funding))
                vectors = json.loads(subprocess.check_output(
                    [exporter, str(funding_file)], text=True, timeout=120,
                ))
                assert manifest(vectors) == manifest(initial), "funding changed vault terms"
                transactions = vectors["transactions"]
                names = ("trigger_cancel", "cancel", "trigger_complete", "complete")
                decoded = {name: rpc("decoderawtransaction", transactions[name]) for name in names}
                previous = funding["funding"]
                for index, name in enumerate(names):
                    transaction = decoded[name]
                    assert len(transaction["vin"]) == 2, (name, transaction)
                    outpoints = [f"{input_['txid']}:{input_['vout']}" for input_ in transaction["vin"]]
                    assert outpoints == [previous, funding["sponsors"][index]], (name, outpoints)
                    previous = f"{transaction['txid']}:0"
                    outputs = transaction["vout"]
                    assert len(outputs) == (2 if name.startswith("trigger") else 1), name
                    expected = vectors["destination"] if name == "complete" else vectors["funding"]
                    assert outputs[0]["scriptPubKey"]["address"] == expected["address"], name
                    assert satoshis(outputs[0]["value"]) == expected["amount_sats"], name
                    if name.startswith("trigger"):
                        marker_address = outputs[1]["scriptPubKey"]["address"]
                        if name == "trigger_complete":
                            assert marker_address == vectors["destination"]["address"], name
                        else:
                            assert marker_address != vectors["destination"]["address"], name
                        assert satoshis(outputs[1]["value"]) == 546, name

                def check(name, allowed, reason=None):
                    result = rpc("testmempoolaccept", [transactions[name]])[0]
                    assert result["allowed"] is allowed, (name, result)
                    if reason is not None:
                        assert result.get("reject-reason") == reason, (name, result)
                    if allowed:
                        expected_fee = 1_454 if name.startswith("trigger") else 2_000
                        assert satoshis(result["fees"]["base"]) == expected_fee, (name, result)
                    print(f"{name}: {'accepted' if allowed else result['reject-reason']}", flush=True)
                    return result

                for name in names[:3]:
                    check(name, True)
                    txid = rpc("sendrawtransaction", transactions[name])
                    rpc("generatetoaddress", 1, miner)
                    assert txid == decoded[name]["txid"], name
                    assert rpc("gettxout", txid, 0)["confirmations"] == 1, name
                trigger_height = rpc("getblockcount")
                check("complete", False, "non-BIP68-final")
                # Core checks the next block. The first valid block is H+20,
                # so admission begins when the tip reaches H+19.
                rpc("generatetoaddress", delay - 2, miner)
                assert rpc("getblockcount") == trigger_height + delay - 2
                check("complete", False, "non-BIP68-final")
                rpc("generatetoaddress", 1, miner)
                for name in ("complete_changed_after_signing", "complete_without_owner"):
                    failure = check(name, False)
                    assert "script" in failure["reject-reason"].lower(), (name, failure)
                check("complete", True)
                complete_txid = rpc("sendrawtransaction", transactions["complete"])
                rpc("generatetoaddress", 1, miner)
                assert rpc("getblockcount") == trigger_height + delay
                assert rpc("gettxout", decoded["trigger_complete"]["txid"], 0) is None
                payout = rpc("gettxout", complete_txid, 0)
                assert payout["confirmations"] == 1, payout
                assert payout["scriptPubKey"]["address"] == vectors["destination"]["address"], payout
                assert satoshis(payout["value"]) == 100_000, payout
                for name in ("trigger_cancel", "trigger_complete"):
                    marker = rpc("gettxout", decoded[name]["txid"], 1)
                    expected_marker = decoded[name]["vout"][1]["scriptPubKey"]["address"]
                    assert marker["scriptPubKey"]["address"] == expected_marker, marker
                    assert satoshis(marker["value"]) == 546, marker
                print("Cancellation preserved the vault, then the full principal withdrew "
                      "after 20 blocks. Core checked Taproot, owner/oracle/sponsor "
                      "signatures and CSV; WASM checked the CAT-style covenant.")
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
