#!/usr/bin/env python3
"""Check compilation and mock funding through the real CLI/WASM workflow."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
cli = Path(sys.argv[1]).resolve()
modules = Path(sys.argv[2]).resolve()
vectors = ROOT / "contrib" / "vectors"
# Cross-module calls compile several Rust modules from authenticated source.
# This bounds the whole CLI request, including native compilation outside fuel.
REQUEST_TIMEOUT_SECONDS = 180

with tempfile.TemporaryDirectory(prefix="sapio-wasm-") as workspace:
    def request(command, module=None, parameters=None, extra_args=()):
        args = [str(cli), "--config", str(vectors / "basic_config.json"),
                "contract", command]
        if module is not None:
            args.extend(["--workspace", workspace, "--file", str(modules / module)])
        args.extend(extra_args)
        started = time.monotonic()
        result = subprocess.run(
            args,
            input=json.dumps(parameters) if parameters is not None else "",
            text=True, capture_output=True, check=True, timeout=REQUEST_TIMEOUT_SECONDS,
        )
        print(f"WASM {command} {module or ''}: {time.monotonic() - started:.2f}s",
              file=sys.stderr)
        response = json.loads(result.stdout)["result"]
        if "Err" in response:
            raise RuntimeError(response["Err"])
        return response["Ok"]

    expected = json.loads((vectors / "clause_output.json").read_text())
    parameters = json.loads((vectors / "clause_input.json").read_text())
    direct = request("create", "sapio_wasm_clause.wasm", parameters)["Call"]["result"]
    assert direct == expected, (direct, expected)

    key = request("load", "sapio_wasm_clause.wasm")["Load"]["key"]
    parameters = json.loads(
        (vectors / "trampoline_clause_input.json").read_text().replace("TEMPLATE_ARG_A", key)
    )
    indirect = request("create", "sapio_wasm_clause_trampoline.wasm", parameters)["Call"]["result"]
    assert indirect == expected, (indirect, expected)

    owner = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
    content_type = b"application/octet-stream"
    body = b"Z" * 521
    parameters = {
        "arguments": {
            "owner": owner,
            "content_type": content_type.decode("ascii"),
            "data": list(body),
        },
        "context": {
            "amount": 10_000,
            "network": "Regtest",
            "ordinals_info": [[0, 10_000]],
        },
    }
    inscription = request(
        "create", "sapio_wasm_ordinal_inscription.wasm", parameters
    )["Call"]["result"]
    descriptor = inscription["known_descriptor"]["XOnly"]
    # The body crosses the 520-byte push boundary, inside a false Ord envelope.
    envelope = (
        b"\x00\x63\x03ord\x01\x01" + bytes([len(content_type)]) + content_type
        + b"\x00\x4d\x08\x02" + body[:520] + b"\x01" + body[520:] + b"\x68"
    )
    assert envelope.hex() in descriptor, descriptor
    assert owner in descriptor, descriptor
    templates = list(inscription["suggested_template_hash_to_template_map"].values())
    assert len(templates) == 1, templates
    outputs = templates[0]["transaction_literal"]["output"]
    assert len(outputs) == 1 and outputs[0]["value"] == 9_500, outputs
    bound = request("bind", parameters=inscription, extra_args=["--mock"])["Bind"]["program"]
    root = inscription["root_path"]
    template_hash = next(iter(inscription["suggested_template_hash_to_template_map"]))
    child = f"{root}/@suggested/{template_hash}/#0"
    assert set(bound) == {root, f"{root}/@funding", child}, bound
    assert bound[root]["source_path"] == root, bound[root]
    assert "source_path" not in bound[f"{root}/@funding"], bound
    assert len(bound[root]["txs"]) == 1 and not bound[child]["txs"], bound
    print("WASM direct, cross-module, inscription compilation and mock binding passed")
