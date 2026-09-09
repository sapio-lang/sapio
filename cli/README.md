# Sapio Command Line Interface (CLI)

The Sapio CLI is a utility tool for using different software components in
the Sapio Project.

You can use the Sapio CLI to build contracts and run other programs.

Sapio CLI reads/writes local project directories for "org.judica.sapio-cli"
based on your local system preferences. See
https://docs.rs/directories/3.0.1/directories/ for more information.

# Config

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
    },
    "plugin_map": {
      "example": "95db1a828dd1c9ab18d431eda9f99af46b9913e818277278a60708012f1d41b3"
    }
  }
}
```

Run `sapio-cli configure wizard --write` to create a configuration. The wizard
requires an explicit covenant mode. Only one network may be active at a time,
but each network can have a defined configuration.

The command line may be used to specify a different configuration.

Every network configuration and Studio request context must include `covenant`.
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

This configuration migration is mandatory. Replace `emulator_nodes` in network
configurations and `emulator` in Studio contexts with the tagged `covenant` field.
Remove `enabled`; missing, null and legacy-only settings fail instead of choosing
native CTV implicitly. A signer configuration or connection error never switches
modes.

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

The plugin_map parameter is used to map human readable names to keys for a
plugin (you can see a plugin's key with the `cli contract load` command).
This enables contracts plugins to be dynamically linked to one another per a
user's preferences.
