#!/usr/bin/env python3
"""Confirm the linked LayerTree MID lifecycle on an isolated regtest node.

Usage: check_layertree.py BITCOIND BITCOIN_CLI LAYERTREE_VECTORS
Build the exporter with `cargo build --locked -p sapio_integration_tests
--example layertree_vectors`. The exporter executes the committed scripts
before signing. Bitcoin Core checks the resulting native oracle and sponsor
witnesses, transaction policy, conservation, and lease finality.
"""

import argparse
from decimal import Decimal
import json
from pathlib import Path
import socket
import subprocess
import tempfile


OPERATIONS = (
    "join", "graft", "rewrite", "update", "split", "claim",
    "hashlocked_claim", "exit", "paired_exit", "cohort_eviction", "eviction",
)
PAYOUT_COUNTS = {
    "exit": 1, "paired_exit": 2, "cohort_eviction": 2, "eviction": 1,
}


def satoshis(value):
    amount = Decimal(str(value)) * 100_000_000
    assert amount == amount.to_integral_value(), value
    return int(amount)


def manifest(document):
    terms = {key: document[key] for key in (
        "profile", "initial_address", "initial_principal", "sponsor_address",
        "final_principal", "remaining_members",
    )}
    terms["steps"] = [
        {key: value for key, value in row.items() if key not in ("txid", "transaction")}
        for row in document["steps"]
    ]
    return terms


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
    assert initial["initial_principal"] == 120_000, initial
    assert initial["final_principal"] == 75_000, initial
    assert initial["remaining_members"] == 3, initial
    assert tuple(row["operation"] for row in initial["steps"]) == OPERATIONS, initial
    assert all(row["fee_sats"] == 2_000 for row in initial["steps"]), initial

    with tempfile.TemporaryDirectory(prefix="sapio-layertree-") as directory:
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
                    command.append("-rpcwallet=layertree")
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
                rpc("createwallet", "layertree")
                miner = rpc("getnewaddress", "", "bech32m", wallet=True)
                rpc("generatetoaddress", 101, miner)

                def fund(address, sats):
                    assert sats > 0, sats
                    amount = f"{sats // 100_000_000}.{sats % 100_000_000:08d}"
                    txid = rpc("sendtoaddress", address, amount, wallet=True)
                    raw = rpc("gettransaction", txid, wallet=True)["hex"]
                    decoded = rpc("decoderawtransaction", raw)
                    matches = [output for output in decoded["vout"]
                               if output["scriptPubKey"].get("address") == address]
                    assert len(matches) == 1, matches
                    assert satoshis(matches[0]["value"]) == sats, matches
                    return f"{txid}:{matches[0]['n']}"

                funding = {
                    "funding": fund(initial["initial_address"], initial["initial_principal"]),
                    "sponsors": [fund(initial["sponsor_address"], row["sponsor_sats"])
                                 for row in initial["steps"]],
                }
                assert len(set([funding["funding"], *funding["sponsors"]])) == 12, funding
                rpc("generatetoaddress", 1, miner)
                funding_file = data / "funding.json"
                funding_file.write_text(json.dumps(funding))
                vectors = json.loads(subprocess.check_output(
                    [exporter, str(funding_file)], text=True, timeout=120,
                ))
                assert manifest(vectors) == manifest(initial), "funding changed LayerTree terms"

                previous = funding["funding"]
                principal = initial["initial_principal"]
                previous_script = rpc("getaddressinfo", initial["initial_address"], wallet=True)["scriptPubKey"]
                for index, row in enumerate(vectors["steps"]):
                    name = row["operation"]
                    transaction = rpc("decoderawtransaction", row["transaction"])
                    assert transaction["txid"] == row["txid"], name
                    assert len(transaction["vin"]) == 2, (name, transaction)
                    outpoints = [f"{input_['txid']}:{input_['vout']}" for input_ in transaction["vin"]]
                    assert outpoints == [previous, funding["sponsors"][index]], (name, outpoints)
                    outputs = transaction["vout"]
                    assert len(outputs) == 1 + PAYOUT_COUNTS.get(name, 0), (name, outputs)
                    assert row["old_principal"] == principal, (name, row)
                    assert satoshis(outputs[0]["value"]) == row["new_principal"], (name, row)
                    script = outputs[0]["scriptPubKey"]
                    assert script["type"] == "witness_v1_taproot", (name, script)
                    assert script["hex"] != previous_script, (name, script)
                    paid = sum(satoshis(output["value"]) for output in outputs[1:])
                    if name in PAYOUT_COUNTS:
                        assert paid == principal - row["new_principal"], (name, row, paid)
                        assert all(output["scriptPubKey"]["type"] == "witness_v1_taproot"
                                   for output in outputs[1:]), (name, outputs)
                    assert principal + row["sponsor_sats"] - sum(
                        satoshis(output["value"]) for output in outputs
                    ) == row["fee_sats"], (name, transaction)

                    # With a future lease, exercise native finality at the
                    # preceding height, then admit the first valid block.
                    lock_time = transaction["locktime"]
                    height = rpc("getblockcount")
                    if lock_time > height:
                        if lock_time > height + 1:
                            rpc("generatetoaddress", lock_time - height - 1, miner)
                        rejected = rpc("testmempoolaccept", [row["transaction"]])[0]
                        assert rejected["allowed"] is False, (name, rejected)
                        assert rejected.get("reject-reason") == "non-final", (name, rejected)
                        print(f"{name}: non-final at tip {lock_time - 1}", flush=True)
                        rpc("generatetoaddress", 1, miner)

                    accepted = rpc("testmempoolaccept", [row["transaction"]])[0]
                    assert accepted["allowed"] is True, (name, accepted)
                    assert satoshis(accepted["fees"]["base"]) == row["fee_sats"], (name, accepted)
                    txid = rpc("sendrawtransaction", row["transaction"])
                    assert txid == transaction["txid"], (name, txid)
                    rpc("generatetoaddress", 1, miner)
                    old_txid, old_index = previous.split(":")
                    assert rpc("gettxout", old_txid, old_index) is None, name
                    sponsor_txid, sponsor_index = funding["sponsors"][index].split(":")
                    assert rpc("gettxout", sponsor_txid, sponsor_index) is None, name
                    state = rpc("gettxout", txid, 0)
                    assert state["confirmations"] == 1, (name, state)
                    assert state["scriptPubKey"]["hex"] == script["hex"], (name, state)
                    assert satoshis(state["value"]) == row["new_principal"], (name, state)
                    for output in outputs[1:]:
                        payout = rpc("gettxout", txid, output["n"])
                        assert payout["confirmations"] == 1, (name, payout)
                        assert payout["scriptPubKey"]["hex"] == output["scriptPubKey"]["hex"], (name, payout)
                        assert satoshis(payout["value"]) == satoshis(output["value"]), (name, payout)
                    previous = f"{txid}:0"
                    previous_script = script["hex"]
                    principal = row["new_principal"]
                    print(f"{name}: confirmed, tree principal {principal} sat", flush=True)

                assert principal == vectors["final_principal"] == 75_000, vectors
                print("Eleven linked LayerTree transitions confirmed with exact payouts "
                      "and exogenous fees. The remaining three members retain 75000 sat.")
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
