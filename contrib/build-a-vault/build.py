#!/usr/bin/env python3
"""Build typed custody blocks, Variables and reusable Studio patches."""

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


CONTEXT = {"amount": 100_000, "network": "Regtest", "lowering": "Native"}


class Recipe:
    def __init__(self, name, keys, schemas, constructor):
        self.name = name
        self.keys = keys
        self.schemas = schemas
        self.constructor = constructor
        self.nodes = []
        self.connections = []

    def add(self, name, module, arguments, column, row):
        self.nodes.append({
            "kind": "module", "id": name, "moduleKey": self.keys[module],
            "arguments": arguments,
            "position": {"x": column * 420, "y": row * 270},
        })
        return name

    def variable(self, name, label, module, arguments, column, row):
        # Public constructors validate the teaching data once. A Variable then
        # emits that literal directly; Studio does not launch its constructor.
        self.nodes.append({
            "kind": "variable", "id": name, "name": label,
            "type": {"schema": copy.deepcopy(self.schemas[module]["returns"])},
            "value": self.constructor(module, arguments),
            "position": {"x": column * 420, "y": row * 270},
        })
        return name

    def wire(self, source, target, field, source_path=""):
        self.connections.append({
            "id": f"{source}-{target}-{field.replace('/', '-')}",
            "kind": "value", "source": source, "sourcePath": source_path,
            "target": target, "targetPath": "/" + field,
        })

    def patch(self):
        builder = self.nodes[-1]
        return {
            "version": 2,
            "nodes": self.nodes + [{
                "kind": "output", "id": "contract-output", "name": "contract",
                "position": {"x": builder["position"]["x"] + 420, "y": builder["position"]["y"]},
            }],
            "connections": self.connections + [{
                "id": builder["id"] + "-contract-output", "kind": "value",
                "source": builder["id"], "sourcePath": "",
                "target": "contract-output", "targetPath": "",
            }],
            "context": copy.deepcopy(CONTEXT),
        }


def recipes(keys, schemas, identity, constructor):
    public = identity["keys"]
    hot_address, cold_address = identity["addresses"]
    result = []
    for quorum in [False, True]:
        r = Recipe("quorum-treasury" if quorum else "fixed-vault", keys, schemas, constructor)
        if quorum:
            r.variable("hot", "Hot authorization", "quorum", {"threshold": 2, "keys": public[:3]}, 0, 0)
        else:
            r.variable("hot", "Hot authorization", "signer", {"key": public[0]}, 0, 0)
        r.variable("cold", "Recovery authorization", "signer", {"key": public[3]}, 0, 2)
        r.variable("wait", "Waiting period", "block-delay", {"blocks": 144}, 0, 1)
        r.variable("hot-destination", "Withdrawal destination", "destination", {"address": hot_address}, 0, 3)
        r.variable("cold-destination", "Recovery destination", "destination", {"address": cold_address}, 0, 4)
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

    r = Recipe("delayed-wallet", keys, schemas, constructor)
    r.variable("hot", "Hot authorization", "signer", {"key": public[0]}, 0, 0)
    r.variable("cold", "Recovery authorization", "quorum", {"threshold": 2, "keys": public[2:]}, 0, 2)
    r.variable("wait", "Waiting period", "block-delay", {"blocks": 1008}, 0, 1)
    r.add("wallet", "delayed-wallet", {}, 1, 1)
    for source, field in [("hot", "hot"), ("cold", "recovery"), ("wait", "delay")]:
        r.wire(source, "wallet", field)
    result.append(r)

    for withdrawal in [100_000, 60_000]:
        r = Recipe("op-vault" if withdrawal == 100_000 else "op-vault-revault", keys, schemas, constructor)
        r.variable("hot", "Hot authorization", "quorum", {"threshold": 2, "keys": public[:3]}, 0, 0)
        r.variable("cold", "Recovery authorization", "signer", {"key": public[3]}, 0, 2)
        r.variable("wait", "Waiting period", "block-delay", {"blocks": 144}, 0, 1)
        r.variable("cold-destination", "Recovery destination", "destination", {"address": cold_address}, 0, 3)
        r.add("recovery", "recovery", {}, 1, 2)
        r.variable("oracle", "Emulation root", "emulation-oracle", {"xpub": identity["oracle_xpub"]}, 1, 0)
        r.variable("withdrawal-destination", "Withdrawal destination", "destination", {"address": hot_address}, 1, 3)
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


def read_pointer(value, pointer):
    for part in pointer.removeprefix("/").split("/") if pointer else []:
        token = part.replace("~1", "/").replace("~0", "~")
        value = value[int(token)] if isinstance(value, list) else value[token]
    return value


def write_pointer(arguments, pointer, value):
    if not pointer:
        return copy.deepcopy(value)
    cursor = arguments
    parts = [part.replace("~1", "/").replace("~0", "~") for part in pointer[1:].split("/")]
    for part in parts[:-1]:
        cursor = cursor.setdefault(part, {})
    cursor[parts[-1]] = copy.deepcopy(value)
    return arguments


def execute_patch(patch, invoke, context, parameters=None, selected_output=None):
    """Evaluate these generated, dependency-ordered graphs using the real CLI."""
    values = {}
    parameters = parameters or {}
    for node in patch["nodes"]:
        if node["kind"] == "variable":
            value = copy.deepcopy(node["value"])
        elif node["kind"] == "parameter":
            value = copy.deepcopy(parameters[node["name"]] if node["name"] in parameters else node["default"])
        elif node["kind"] == "output":
            incoming = [wire for wire in patch["connections"] if wire["target"] == node["id"]]
            assert len(incoming) == 1 and incoming[0]["targetPath"] == "", "Output needs one whole-value connection"
            assert not any(wire["source"] == node["id"] for wire in patch["connections"]), "Output cannot have outgoing connections"
            wire = incoming[0]
            value = read_pointer(values[wire["source"]], wire["sourcePath"])
        else:
            arguments = copy.deepcopy(node.get("arguments", {}))
            for wire in patch["connections"]:
                if wire["target"] == node["id"]:
                    source = read_pointer(values[wire["source"]], wire["sourcePath"])
                    arguments = write_pointer(arguments, wire["targetPath"], source)
            value = (
                execute_patch(node["patch"], invoke, context, arguments)
                if node["kind"] == "subpatch"
                else invoke(node["moduleKey"], arguments, context)
            )
        values[node["id"]] = value
    outputs = {
        node["name"]: values[node["id"]]
        for node in patch["nodes"] if node["kind"] == "output"
    }
    if selected_output is None:
        return outputs
    assert any(node["kind"] == "output" and node["id"] == selected_output for node in patch["nodes"]), "Selected result must be an Output terminal"
    return values[selected_output]


def reusable_recovery(fixed_patch, keys, schemas):
    """Expose a composer's two inputs without changing its meaning or identity."""
    definition = {
        "version": 2,
        "nodes": [
            {
                "kind": "parameter", "id": name, "name": name,
                "type": {"schema": copy.deepcopy(schemas[module]["returns"])},
                "position": {"x": 0, "y": index * 270},
            }
            for index, (name, module) in enumerate([
                ("authorization", "signer"), ("destination", "destination"),
            ])
        ] + [{
            "kind": "module", "id": "recovery", "moduleKey": keys["recovery"],
            "arguments": {}, "position": {"x": 420, "y": 100},
        }, {
            "kind": "output", "id": "recovery-output", "name": "recovery",
            "position": {"x": 840, "y": 100},
        }],
        "connections": [
            {"id": name + "-recovery", "kind": "value", "source": name,
             "sourcePath": "", "target": "recovery", "targetPath": "/" + name}
            for name in ["authorization", "destination"]
        ] + [{
            "id": "recovery-output", "kind": "value", "source": "recovery",
            "sourcePath": "", "target": "recovery-output", "targetPath": "",
        }],
    }
    example = copy.deepcopy(fixed_patch)
    for index, node in enumerate(example["nodes"]):
        if node["id"] == "recovery":
            example["nodes"][index] = {
                "kind": "subpatch", "id": "recovery", "name": "Recovery policy",
                "patch": copy.deepcopy(definition), "arguments": {},
                "position": node["position"],
            }
    for wire in example["connections"]:
        if wire["source"] == "recovery":
            wire["sourcePath"] = "/recovery"
    return {**definition, "context": copy.deepcopy(CONTEXT)}, example


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
    modules, keys, schemas = [], {}, {}
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
        schemas[name] = api
        save(output / "schemas" / f"{name}.json", api)
        print(f"Loaded {name}: {key}", flush=True)
    manifest = {"modules": modules, "recipes": [], "samples": [], "reusable": []}
    identity = json.loads((HERE / "demo-identity.json").read_text())

    def invoke(key, arguments, context):
        return json.loads(run(
            [cli, "contract", "create", "--workspace", workspace, "--key", key],
            {"arguments": arguments, "context": context},
        ))

    samples = {}

    def constructor(module, arguments):
        identity = (module, json.dumps(arguments, sort_keys=True))
        if identity not in samples:
            value = invoke(keys[module], arguments, CONTEXT)
            samples[identity] = value
            manifest["samples"].append({
                "name": f"{module}-{len(samples)}", "module": keys[module],
                "arguments": arguments, "value": value,
                "context": copy.deepcopy(CONTEXT),
            })
        return copy.deepcopy(samples[identity])

    fixed_patch = fixed_artifact = None
    for recipe in recipes(keys, schemas, identity, constructor):
        patch = recipe.patch()
        artifact = execute_patch(patch, invoke, patch["context"], selected_output="contract-output")
        if recipe.name == "fixed-vault":
            fixed_patch, fixed_artifact = patch, artifact
        explanation = json.loads(run([cli, "contract", "explain", "--json"], artifact))
        save(output / f"{recipe.name}.patch.json", patch)
        save(output / f"{recipe.name}.artifact.json", artifact)
        save(output / f"{recipe.name}.explanation.json", explanation)
        manifest["recipes"].append({
            "name": recipe.name, "patch": f"{recipe.name}.patch.json",
            "artifact": f"{recipe.name}.artifact.json", "output": "contract-output",
        })
        print(f"Compiled {recipe.name}: {len(patch['nodes'])} typed nodes", flush=True)
    definition, reusable = reusable_recovery(fixed_patch, keys, schemas)
    assert execute_patch(reusable, invoke, reusable["context"], selected_output="contract-output") == fixed_artifact, "Reusable recovery changed the fixed vault artifact"
    save(output / "recovery-policy.patch.json", definition)
    save(output / "reusable-recovery.patch.json", reusable)
    manifest["reusable"].append({
        "name": "reusable-recovery", "definition": "recovery-policy.patch.json",
        "patch": "reusable-recovery.patch.json", "artifact": "fixed-vault.artifact.json",
        "output": "contract-output",
    })
    save(output / "manifest.json", manifest)
    print(f"Open patches from {output} in Studio; set workspace to {workspace}.")


if __name__ == "__main__":
    main()
