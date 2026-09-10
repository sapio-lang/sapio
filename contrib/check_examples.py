#!/usr/bin/env python3
"""Require and execute a contract fixture for every WASM workspace module."""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "contrib" / "vectors" / "examples"
INTERFACES = {"batching-trait", "nft-trait"}
SIGNER_CASES = ("treepay", "trampolinepay")
# Each fixture performs two full create requests to check repeatability.
REQUEST_TIMEOUT_SECONDS = 360


def main():
    runner, modules = map(lambda arg: Path(arg).resolve(), sys.argv[1:])
    catalog = json.loads((FIXTURES / "catalog.json").read_text())
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1",
         "--manifest-path", str(ROOT / "plugin-example" / "Cargo.toml")], text=True,
    ))
    workspace = set(metadata["workspace_members"])
    guests, interfaces = {}, set()
    for package in metadata["packages"]:
        if package["id"] not in workspace:
            continue
        directory = Path(package["manifest_path"]).parent.name
        targets = [t for t in package["targets"] if "cdylib" in t["crate_types"]]
        if targets:
            assert len(targets) == 1, package
            guests[directory] = targets[0]["name"] + ".wasm"
        else:
            interfaces.add(directory)
    assert interfaces == INTERFACES, (interfaces, INTERFACES)
    assert {name: case["wasm"] for name, case in catalog.items()} == guests
    cases = [(name, "native") for name in sorted(catalog)]
    cases.extend((name, "signer") for name in SIGNER_CASES)
    assert all(name in catalog for name in SIGNER_CASES)
    failures = []
    for name, mode in cases:
        label = f"{name} [{mode}]"
        started = time.monotonic()
        print(f"Checking WASM {label}...", flush=True)
        with tempfile.TemporaryDirectory(prefix="sapio-example-") as cache:
            try:
                command = [str(runner), str(modules), str(FIXTURES), name, cache]
                if mode == "signer":
                    command.append("signer")
                subprocess.run(
                    command,
                    check=True, timeout=REQUEST_TIMEOUT_SECONDS,
                )
            except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
                failures.append(label)
                print(f"FAILED {label}: {error}", file=sys.stderr)
        print(f"WASM {label}: {time.monotonic() - started:.2f}s", flush=True)
    assert not failures, f"Failing examples: {', '.join(failures)}"
    print(
        f"All {len(guests)} WASM modules checked in {len(cases)} cases "
        f"({len(SIGNER_CASES)} signer propagation cases); "
        f"{len(interfaces)} shared interfaces inventoried"
    )


if __name__ == "__main__":
    main()
