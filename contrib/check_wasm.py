#!/usr/bin/env python3
"""Check direct, cross-module, and inscription compilation through CLI/WASM."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
cli = Path(sys.argv[1]).resolve()
modules = Path(sys.argv[2]).resolve()
vectors = ROOT / "contrib" / "vectors"

with tempfile.TemporaryDirectory(prefix="sapio-wasm-") as workspace:
    def request(command, module, parameters=None):
        result = subprocess.run(
            [str(cli), "--config", str(vectors / "basic_config.json"),
             "contract", command, "--workspace", workspace,
             "--file", str(modules / module)],
            input=json.dumps(parameters) if parameters is not None else "",
            text=True, capture_output=True, check=True, timeout=60,
        )
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
    print("WASM direct, cross-module, and inscription compilation passed")
