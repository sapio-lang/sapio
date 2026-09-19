#!/usr/bin/env python3
"""Execute the generated starter's documented workflow as an external consumer."""

import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile


def run(args, cwd, env):
    return subprocess.run(
        args, cwd=cwd, env=env, text=True, capture_output=True, timeout=1200,
    )


def require_success(result, stage):
    if result.returncode:
        raise RuntimeError(
            f"{stage} failed ({result.returncode})\n"
            f"{result.stdout[-12000:]}\n{result.stderr[-12000:]}"
        )


def main():
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} PATH_TO_SAPIO_CLI")
    cli = Path(sys.argv[1]).resolve(strict=True)
    env = os.environ.copy()
    env["PATH"] = str(cli.parent) + os.pathsep + env.get("PATH", "")

    # The directory is outside Sapio's Cargo workspace. The consumer must
    # supply its own patches, pinned Git dependencies and valid lockfile.
    with tempfile.TemporaryDirectory(prefix="sapio-quickstart-") as temporary:
        directory = Path(temporary)
        project = directory / "first-contract"
        create = [str(cli), "new", str(project), "--name", "quickstart-contract"]
        require_success(run(create, directory, env), "project generation")
        manifest = (project / "Cargo.toml").read_bytes()
        lockfile = (project / "Cargo.lock").read_bytes()
        refused = run(create, directory, env)
        assert refused.returncode == 1, refused
        assert not refused.stdout, refused.stdout
        assert (project / "Cargo.toml").read_bytes() == manifest
        assert (project / "Cargo.lock").read_bytes() == lockfile

        # These are the actual instructions shipped to users, without a
        # parallel copy of the commands that could quietly drift from them.
        blocks = re.findall(r"^```sh\n(.*?)^```$", (project / "README.md").read_text(),
                            re.MULTILINE | re.DOTALL)
        assert len(blocks) == 3, "expected build, completion and rejection shell blocks"
        for title, block, cwd in zip(
            ["build and inspect", "prepare, sign and complete"],
            blocks[:2], [project, project / "demo"],
        ):
            result = run(["bash", "-euo", "pipefail", "-c", block], cwd, env)
            require_success(result, title)
            print(f"Quickstart {title}: passed", flush=True)

        transaction = (project / "demo" / "transaction.hex").read_text().strip()
        assert bytes.fromhex(transaction), "finalization returned an empty transaction"

        evaluator = bytearray((project / "demo" / "pay_at_least.wasm").read_bytes())
        evaluator[-1] ^= 1
        (project / "demo" / "different.wasm").write_bytes(evaluator)
        mismatch = run([
            str(cli), "signer", "program", "--key", "oracle.key",
            "--request", "request.json", "--evaluator", "different.wasm",
            "--output", "mismatched.psbt",
        ], project / "demo", env)
        assert mismatch.returncode == 1, mismatch
        assert "do not match the request's committed evaluator ID" in mismatch.stderr
        assert not (project / "demo" / "mismatched.psbt").exists()

        rejected = run(["bash", "-euo", "pipefail", "-c", blocks[2]], project, env)
        assert rejected.returncode == 1, rejected
        assert "program predicate rejected the transaction" in rejected.stderr, rejected.stderr
        assert (project / "rejected" / "request.json").is_file()
        assert not (project / "rejected" / "response.psbt").exists()
        assert (project / "Cargo.lock").read_bytes() == lockfile
        print("Quickstart below-minimum rejection: passed", flush=True)


if __name__ == "__main__":
    main()
