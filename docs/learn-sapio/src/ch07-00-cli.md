# Sapio Command Line Interface

The maintained starter walkthrough uses the CLI to inspect an artifact, prepare
one branch, export an explicit program request, import its verified response and
complete the retained witness. See the generated project README and the
[current CLI guide](https://github.com/sapio-lang/sapio/blob/master/cli/README.md).

From the Sapio repository root, inspect the available commands with:

```sh
cargo run --locked -p sapio-cli -- --help
cargo run --locked -p sapio-cli -- contract spend --help
```

Artifact inspection and selected-spend commands use local files without loading
wallet or network configuration. `signer program` uses an explicitly supplied
key, request and evaluator. It does not discover oracles or register code from
artifact metadata.

Other commands support compiler plugins, binding and emulator services. Those
workflows have their own configuration and enforcement assumptions; follow the
current CLI guide rather than older Studio or container setup instructions.
