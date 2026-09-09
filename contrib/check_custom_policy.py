#!/usr/bin/env python3
"""Execute Sapio's raw-policy witnesses against isolated Bitcoin Core regtest.

Usage: check_custom_policy.py BITCOIND BITCOIN_CLI CUSTOM_POLICY_VECTORS
The vector executable comes from `cargo build -p sapio --example
custom_policy_vectors`. No existing node, wallet, or user funds are accessed.
"""

import json
from pathlib import Path
import socket
import subprocess
import sys
import tempfile


def main():
    if len(sys.argv) != 4:
        raise SystemExit(__doc__)
    bitcoind, bitcoin_cli, vectors = [str(Path(arg).resolve()) for arg in sys.argv[1:]]
    initial = json.loads(subprocess.check_output([vectors], text=True, timeout=30))
    with tempfile.TemporaryDirectory(prefix="sapio-custom-policy-") as directory:
        data = Path(directory)
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        with (data / "node.log").open("w") as log:
            node = subprocess.Popen(
                [bitcoind, "-regtest", "-server=1", f"-datadir={data}",
                 f"-rpcport={port}", "-listen=0", "-connect=0", "-dnsseed=0",
                 "-discover=0", "-fallbackfee=0.00001", "-printtoconsole=1"],
                stdout=log, stderr=subprocess.STDOUT,
            )

            def rpc(method, *arguments, wallet=False):
                command = [bitcoin_cli, "-regtest", f"-datadir={data}",
                           f"-rpcport={port}", "-rpcwait", "-rpcwaittimeout=30"]
                if wallet:
                    command.append("-rpcwallet=custom-policy")
                command.append(method)
                command.extend(
                    json.dumps(argument, separators=(",", ":"))
                    if isinstance(argument, (dict, list, bool)) else str(argument)
                    for argument in arguments
                )
                output = subprocess.check_output(command, text=True, timeout=40).strip()
                if not output:
                    return None
                try:
                    return json.loads(output)
                except json.JSONDecodeError:
                    return output

            try:
                rpc("getblockchaininfo")
                rpc("createwallet", "custom-policy")
                miner = rpc("getnewaddress", "", "bech32m", wallet=True)
                rpc("generatetoaddress", 101, miner)
                funding = {}
                for group in initial["groups"]:
                    amount = group["funding_amount_sats"]
                    coins = f"{amount // 100_000_000}.{amount % 100_000_000:08d}"
                    txid = rpc("sendtoaddress", group["address"], coins, wallet=True)
                    raw = rpc("gettransaction", txid, wallet=True)["hex"]
                    transaction = rpc("decoderawtransaction", raw)
                    matches = [output for output in transaction["vout"]
                               if output["scriptPubKey"].get("address") == group["address"]]
                    assert len(matches) == 1, (group["name"], matches)
                    funding[group["name"]] = f"{txid}:{matches[0]['n']}"
                rpc("generatetoaddress", 1, miner)
                funding_file = data / "funding.json"
                funding_file.write_text(json.dumps(funding))
                signed = json.loads(subprocess.check_output(
                    [vectors, str(funding_file)], text=True, timeout=30,
                ))
                accepted, rejected = 0, 0
                for group in signed["groups"]:
                    for case in group["cases"]:
                        result = rpc("testmempoolaccept", [case["transaction"]])[0]
                        assert result["allowed"] == case["allowed"], (
                            group["name"], case["name"], result,
                        )
                        if case["allowed"]:
                            accepted += 1
                        else:
                            rejected += 1
                            # A missing funding input would reject everything;
                            # require each negative to reach script validation.
                            assert "script" in result.get("reject-reason", "").lower(), result
                        print(f"{group['name']}/{case['name']}: "
                              f"{'accepted' if result['allowed'] else 'rejected'}", flush=True)
                assert (accepted, rejected) == (2, 10), (accepted, rejected)
                print(f"Bitcoin Core accepted {accepted} valid raw-policy spends and "
                      f"rejected {rejected} invalid spends. Native CTV was not tested.")
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
