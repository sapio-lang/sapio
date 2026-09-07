#!/usr/bin/env python3
"""Check direct and cross-module compilation through the real CLI and WASM ABI."""

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
            text=True, capture_output=True, check=True,
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
    print("WASM direct and cross-module compilation passed")
