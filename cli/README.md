# Sapio Command Line Interface (CLI)

The Sapio CLI creates projects, compiles and inspects contracts, and completes
explicitly selected spends. Run `sapio-cli --help` or a subcommand's `--help`
for its typed arguments. Commands return a nonzero exit status on failure.

## Create your first project

```sh
sapio-cli new my-contract --name my-contract
cd my-contract
cargo test --locked
```

The project includes its pinned dependencies, lockfile, toolchain, evaluator,
contract source and tests. Its README carries the exported demo files through
the complete signing workflow. See the [quickstart](../docs/QUICKSTART.md) to
build the CLI from a source checkout. The starter uses synthetic funding and
public demonstration keys; it performs no wallet activity or broadcasting.

## Compile and inspect compiler plugins

Local module commands do not read runtime `--config`. Supply one source using
`--file` or `--key`. An optional `--workspace DIR` stores cached sources under
`DIR/modules`; `--plugin-map FILE` supplies a JSON object mapping module aliases
to their hashes. Use `configure files --json` to inspect default paths.

```sh
sapio-cli contract api --file contract.wasm
sapio-cli contract create --file contract.wasm --args args.json --output artifact.json
sapio-cli contract explain --file artifact.json
```

`args.json` is the module's `CreateArgs` input, including `arguments` and public
`context.network`, `context.amount` and `context.lowering`. Use the module's API
schema for its exact arguments. Omit `--args`, or use `--args -`, to read stdin.
Creation returns the raw contract artifact, which can feed `explain` directly;
it does not wrap it in a Studio response. `api`, `info`, `logo`, `list` and
`load` likewise return their payloads directly.

Outputs go to stdout unless `--output` names a new file. Existing output files
are never overwritten. Use files for the distinct inputs of multi-input commands.

## Complete a selected spend

`contract spend` resumes one exact spending branch from a trusted artifact,
immutable intent JSON, and current base64 PSBT. It needs no CLI configuration
or coordinator service:

```sh
sapio-cli contract spend prepare --artifact artifact.json --psbt funded.psbt \
  --path key --assets assets.json --evidence evidence.json --output intent.json
sapio-cli contract spend requests --artifact artifact.json --intent intent.json
sapio-cli contract spend apply --artifact artifact.json --intent intent.json \
  --response 0=response.psbt --output current.psbt
sapio-cli contract spend status --artifact artifact.json --intent intent.json \
  --psbt current.psbt
sapio-cli contract spend finalize --artifact artifact.json --intent intent.json \
  --psbt current.psbt --transaction --output transaction.hex
```

Choose each request's signer explicitly. Responses are verified against their
original requests and can arrive out of order. `sign-native --key native.key`
signs only the selected input's native slots. Finalization verifies the selected
witness, other funded inputs and retained fee rules before extraction. Output
files must be new; every resumed command accepts the latest `--psbt` separately
from the unchanged intent. See [spend completion](../docs/SPEND_COMPLETION.md)
for script paths, multiple signers and restarting between operations.

An exported `ProgramSigningRequest` can be evaluated and signed locally using
an explicitly selected oracle key and exact interpreter:

```sh
sapio-cli signer program --key oracle.key --request request.json \
  --evaluator pay_at_least.wasm --output response.psbt
```

The key file uses Sapio's binary Xpriv encoding. Registered evaluators require
the exact supplied file matching the request's committed evaluator ID and ABI.
Inline WASM requests carry their program and must omit `--evaluator`. Evaluation
runs locally; no signer endpoint is selected automatically. The
[starter walkthrough](templates/starter/README.md)
provides matching keys, interpreter, evidence and requests for a synthetic
payment, including a rejected underpayment.

## Explain a contract

Inspect a compiled artifact without a wallet, signer connection, or CLI configuration:

```sh
sapio-cli contract explain --file artifact.json
sapio-cli contract explain --file artifact.json --json
```

The input is the compiled object itself. Local `contract create` output can
be inspected directly through stdin:

```sh
sapio-cli contract create --file contract.wasm --args args.json | sapio-cli contract explain
```

The report validates the complete graph and shows ordered output allocations,
funding roles, reserved fees and explicit fee caps, fixed lock fields, covenant
lowering, program policies, descriptors, and advertised action request schemas.
Each output occurrence has a JSON pointer back to the artifact, so reused child
source paths remain distinguishable. Optional metadata does not create spending
requirements. A suggested transaction remains a proposal; its presence does not
make its outputs covenant-enforced.

Supply a funded PSBT file and an optional public capability inventory to inspect
the actual spending requirements:

```sh
sapio-cli contract explain --file artifact.json \
  --psbt funded.psbt --input 0 --assets capabilities.json --json
```

`funded.psbt` contains a base64 PSBT. `capabilities.json` deserializes as
`emulator_connect::program::SpendAssets`; `{}` declares no available credentials.
For an ordinary signer, an inventory can be as small as
`{"schnorr_keys":["<x-only public key>"]}`. Inventories contain public keys,
hashlock identities, and explicit program/evidence capabilities, not private keys
or evidence bytes.

The spending report lists missing previous outputs, missing assets, selected
witness layouts and known weight bounds. Matching transaction templates also
report actual fees and whether final witness weight is still needed for a fee-rate
check. Invalid funding or a violated fee cap fails before any signing. Planning
does not produce signatures, execute evaluators, prove chain maturity, or broadcast
transactions.

## Bind with an explicit funding source

Binding uses runtime configuration and requires exactly one funding choice:
`--mock`, `--outpoint TXID:VOUT`, `--funding-psbt FILE` or `--wallet-fund`.
Omitting the choice never silently invokes a wallet.

```sh
sapio-cli --config config.json contract bind --artifact artifact.json \
  --mock --output bound.json
```

`--mock` produces synthetic funding. `--outpoint` fetches the specified output
from the configured node. `--funding-psbt` accepts a funding transaction supplied
by the caller. `--wallet-fund` explicitly asks the configured wallet to fund the
contract. The configured covenant mode must match the compiled assumptions.

For a standalone PSBT, `sapio-cli psbt finalize --psbt signed.psbt` returns the
decoded finalized PSBT/transaction as JSON. Omit `--psbt`, or use `--psbt -`, to
read stdin. Use `contract spend finalize` when completing a saved intent so its
selected witness and retained funding rules are checked as well.

## Runtime configuration

A Sapio Config file (on linux at `~/.config/sapio-cli/config.json`) is a valid JSON file that looks like:

```json
{
  "main": null,
  "testnet": null,
  "signet": null,
  "regtest": {
    "active": true,
    "api_node": {
      "url": "http://127.0.0.1:18443",
      "auth": {
        "CookieFile": "/home/<user>/.bitcoin/regtest/.cookie"
      }
    },
    "covenant": {
      "mode": "signer_emulation",
      "emulators": [
        [
          "tpubD6NzVbkrYhZ4Wf398td3H8YhWBsXx9Sxa4W3cQWkNW3N3DHSNB2qtPoUMXrA6JNaPxodQfRpoZNE5tGM9iZ4xfUEFRJEJvfs8W5paUagYCE",
          "127.0.0.1:8367"
        ]
      ],
      "threshold": 1,
      "request_timeout_secs": 30
    }
  }
}
```

Run `sapio-cli configure wizard --write` to create a configuration. The wizard
requires an explicit covenant mode. Only one network may be active at a time,
but each network can have a defined configuration.

The command line may be used to specify a different configuration.

Every network configuration and Studio `Bind` command must include `covenant`.
This selects runtime binding and signing assumptions. The `signer_emulation`
mode relies on the configured signers' security and availability. Replace the
example public key and address with your own signer's values. Multiple peers support a
threshold policy; see [CTV emulators](../ctv_emulators/README.md).

For research on a chain assumed to enforce native CTV, select explicitly:

```json
{"covenant": {"mode": "native_ctv_research"}}
```

This is an operator assumption, not node capability detection. Sapio does not
infer enforcement from a network name. Ordinary `signer_emulation` rejects known
spending scripts containing native CTV before funding or binding, including native checks
written directly in guards or raw policies. Binding also checks that the
configured backend reproduces the policies derived from the artifact's recorded
covenant requirements; changing
signer keys or thresholds requires recompilation.

Contracts combining signer-emulated templates with direct native CTV guards can
use `signer_emulation_with_native_ctv_research`. It takes the same `emulators`,
`threshold` and `request_timeout_secs` fields as signer mode and derives the same
signer policies. It additionally records the operator's explicit native CTV
assumption, permitting those mixed scripts at the funding boundary. It does not
replace signer checks with native CTV or relax the recorded-policy comparison.

Network configurations use the tagged `covenant` field. In the Studio protocol,
runtime covenant selection belongs to the `Bind` command, not its shared
`context`; module inspection and compilation need no runtime policy. Missing or
invalid binding policy fails instead of choosing native CTV implicitly. A signer
configuration or connection error never switches modes.

Compilation uses the mandatory `context.lowering` in the create arguments,
independently of these runtime settings:

```json
{"arguments": {}, "context": {"network": "Regtest", "amount": 1000, "lowering": "Native"}}
```

For emulation, set `lowering` to
`{"CtvEmulation":{"signers":["<extended public key>"],"threshold":1}}`.
Only explicitly emulatable predicates follow this plan; direct native clauses
and raw scripts retain their meaning. The public roots and threshold determine
the compiled policy without DNS, connections or signer callbacks. Nested modules
receive the same explicit plan. Creating contracts and inspecting module metadata
never resolve runtime signer settings. At binding, the configured signer must
reproduce the policies selected by the recorded plan.

Old create requests lacking `context.lowering` fail decoding. Rebuild old WASM
plugins: compilation hosts no longer expose the signer-policy or signing imports.

`request_timeout_secs` defaults to 30. Resolving the complete peer list has one
deadline; each peer signing request has a separate deadline covering the wait
for a previous request, connecting, and the complete exchange. A federation
contacts peers sequentially, so its total duration can span multiple deadlines.
The threshold must be between one and the number of peers.
Failed or interrupted exchanges close the
connection so that the next request can reconnect.
The DNS deadline stops the async wait; blocking system resolver work can
continue and delay process shutdown.

Run your own emulator with a seed file and a listening address:

```sh
sapio-cli --config config.json emulator server seed.bin 127.0.0.1:8367 \
  --request-timeout-secs 30 --max-connections 64
```

Both limits must be positive. They default to 30 seconds per request and 64
admitted connections. Idle connections expire under the same deadline; partial
progress does not restart it. At capacity, new connections wait in the operating
system's backlog. These limits bound I/O waits and admitted connections, not
synchronous signing CPU time.

The server prints one JSON readiness record after binding, including the actual
address, public key and limits. Port `0` requests an automatically assigned port.
Starting a server does not resolve the configured remote emulator peers. The
old `--sync` debug mode has been removed.

Use `sapio-cli emulator get-key --psbt funded.psbt` to inspect the configured
CTV signing condition. The CTV service and `signer program` implement different
protocols; select the command matching the request you prepared.

## Studio protocol

`sapio-cli studio server --stdin` retains the JSON request/response envelopes
for Studio clients. `sapio-cli studio schemas` describes the current protocol.
The shared context contains module location, cache path, network and optional
module aliases. Runtime covenant settings belong only to `Bind`.

The old ignored `--base64_psbt`, debug switches and Studio `--interface` option
are not part of the current CLI. PSBT file arguments carry base64; the supported
Studio transport is stdin/stdout. Invalid flags fail during argument parsing.
