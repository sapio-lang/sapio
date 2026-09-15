#!/usr/bin/env python3
"""Build ten WASM blocks and generate executable Studio custody patches."""

import argparse
import copy
import json
from pathlib import Path
import shutil
import subprocess

HERE = Path(__file__).resolve().parent
MODULES = [
    "signer", "quorum", "block-delay", "destination", "recovery", "release",
    "fixed-vault", "delayed-wallet", "emulation-oracle", "op-vault",
]


def run(command, value=None):
    result = subprocess.run(
        [str(part) for part in command],
        input=None if value is None else json.dumps(value),
        text=True, capture_output=True, timeout=1200, check=False,
    )
    if result.returncode:
        raise RuntimeError(
            f"Command failed: {' '.join(map(str, command))}\n{result.stderr[-12000:]}"
        )
    return result.stdout


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n")


class Recipe:
    def __init__(self, name, keys):
        self.name = name
        self.keys = keys
        self.nodes = []
        self.connections = []

    def add(self, name, module, arguments, column, row):
        self.nodes.append({
            "id": name, "moduleKey": self.keys[module], "arguments": arguments,
            "position": {"x": column * 330, "y": row * 210},
        })
        return name

    def wire(self, source, target, field):
        self.connections.append({
            "id": f"{source}-{target}-{field.replace('/', '-')}",
            "kind": "value", "source": source, "sourcePath": "",
            "target": target, "targetPath": "/" + field,
        })

    def patch(self):
        return {
            "version": 1, "nodes": self.nodes, "connections": self.connections,
            "context": {"amount": 100_000, "network": "Regtest", "lowering": "Native"},
        }


def recipes(keys, identity):
    public = identity["keys"]
    hot_address, cold_address = identity["addresses"]
    result = []
    for quorum in [False, True]:
        r = Recipe("quorum-treasury" if quorum else "fixed-vault", keys)
        if quorum:
            r.add("hot", "quorum", {"threshold": 2, "keys": public[:3]}, 0, 0)
        else:
            r.add("hot", "signer", {"key": public[0]}, 0, 0)
        r.add("cold", "signer", {"key": public[3]}, 0, 2)
        r.add("wait", "block-delay", {"blocks": 144}, 0, 1)
        r.add("hot-destination", "destination", {"address": hot_address}, 0, 3)
        r.add("cold-destination", "destination", {"address": cold_address}, 0, 4)
        r.add("release", "release", {}, 1, 0)
        r.add("recovery", "recovery", {}, 1, 2)
        r.add("vault", "fixed-vault", {"fee_sats": 500}, 2, 1)
        for source, target, field in [
            ("hot", "release", "authorization"), ("wait", "release", "delay"),
            ("hot-destination", "release", "destination"),
            ("cold", "recovery", "authorization"),
            ("cold-destination", "recovery", "destination"),
            ("hot", "vault", "trigger"), ("release", "vault", "release"),
            ("recovery", "vault", "recovery"),
        ]:
            r.wire(source, target, field)
        result.append(r)

    r = Recipe("delayed-wallet", keys)
    r.add("hot", "signer", {"key": public[0]}, 0, 0)
    r.add("cold", "quorum", {"threshold": 2, "keys": public[2:]}, 0, 2)
    r.add("wait", "block-delay", {"blocks": 1008}, 0, 1)
    r.add("wallet", "delayed-wallet", {}, 1, 1)
    for source, field in [("hot", "hot"), ("cold", "recovery"), ("wait", "delay")]:
        r.wire(source, "wallet", field)
    result.append(r)

    for withdrawal in [100_000, 60_000]:
        r = Recipe("op-vault" if withdrawal == 100_000 else "op-vault-revault", keys)
        r.add("hot", "quorum", {"threshold": 2, "keys": public[:3]}, 0, 0)
        r.add("cold", "signer", {"key": public[3]}, 0, 2)
        r.add("wait", "block-delay", {"blocks": 144}, 0, 1)
        r.add("cold-destination", "destination", {"address": cold_address}, 0, 3)
        r.add("recovery", "recovery", {}, 1, 2)
        r.add("oracle", "emulation-oracle", {"xpub": identity["oracle_xpub"]}, 1, 0)
        r.add("withdrawal-destination", "destination", {"address": hot_address}, 1, 3)
        r.add("vault", "op-vault", {"proposal": {"withdrawal_sats": withdrawal}}, 2, 1)
        for source, target, field in [
            ("cold", "recovery", "authorization"),
            ("cold-destination", "recovery", "destination"),
            ("hot", "vault", "trigger"), ("wait", "vault", "delay"),
            ("recovery", "vault", "recovery"), ("oracle", "vault", "oracle"),
            ("withdrawal-destination", "vault", "proposal/destination"),
        ]:
            r.wire(source, target, field)
        result.append(r)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--workspace", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--target-dir", type=Path, default=HERE / "target")
    parser.add_argument("--skip-build", action="store_true")
    args = parser.parse_args()
    cli = args.cli.resolve(strict=True)
    workspace, output, target = (p.resolve() for p in [args.workspace, args.output, args.target_dir])
    if not args.skip_build:
        print("Building ten custody modules…", flush=True)
        command = [
            "cargo", "build", "--manifest-path", HERE / "Cargo.toml",
            "--locked", "--release", "--target", "wasm32-unknown-unknown",
            "--target-dir", target, "--workspace", "--lib",
        ]
        run(command)
    output.mkdir(parents=True, exist_ok=True)
    (output / "modules").mkdir(exist_ok=True)
    modules, keys = [], {}
    for name in MODULES:
        filename = "build_a_vault_" + name.replace("-", "_") + ".wasm"
        source = target / "wasm32-unknown-unknown" / "release" / filename
        destination = output / "modules" / filename
        shutil.copyfile(source, destination)
        loaded = json.loads(run([cli, "contract", "load", "--workspace", workspace, "--file", destination]))
        key = loaded["key"]
        assert isinstance(key, str) and len(key) == 64, loaded
        keys[name] = key
        modules.append({"name": name, "key": key, "path": f"modules/{filename}"})
        api = json.loads(run([cli, "contract", "api", "--workspace", workspace, "--key", key]))
        save(output / "schemas" / f"{name}.json", api)
        print(f"Loaded {name}: {key}", flush=True)
    manifest = {"modules": modules, "recipes": []}
    identity = json.loads((HERE / "demo-identity.json").read_text())
    for recipe in recipes(keys, identity):
        patch = recipe.patch()
        values = {}
        for node in patch["nodes"]:
            arguments = copy.deepcopy(node["arguments"])
            for wire in patch["connections"]:
                if wire["target"] != node["id"]:
                    continue
                cursor = arguments
                parts = wire["targetPath"][1:].split("/")
                for part in parts[:-1]:
                    cursor = cursor.setdefault(part, {})
                cursor[parts[-1]] = copy.deepcopy(values[wire["source"]])
            value = {"arguments": arguments, "context": patch["context"]}
            response = run([cli, "contract", "create", "--workspace", workspace, "--key", node["moduleKey"]], value)
            values[node["id"]] = json.loads(response)
        artifact = values[patch["nodes"][-1]["id"]]
        explanation = json.loads(run([cli, "contract", "explain", "--json"], artifact))
        save(output / f"{recipe.name}.patch.json", patch)
        save(output / f"{recipe.name}.artifact.json", artifact)
        save(output / f"{recipe.name}.explanation.json", explanation)
        manifest["recipes"].append({
            "name": recipe.name, "patch": f"{recipe.name}.patch.json",
            "artifact": f"{recipe.name}.artifact.json",
        })
        print(f"Compiled {recipe.name}: {len(patch['nodes'])} connected blocks", flush=True)
    save(output / "manifest.json", manifest)
    print(f"Open patches from {output} in Studio; set workspace to {workspace}.")


if __name__ == "__main__":
    main()
