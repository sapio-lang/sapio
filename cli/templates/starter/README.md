# Your first Sapio contract

This project proposes a payment and asks an explicitly chosen WASM evaluator and
oracle key to authorize it. It uses synthetic funding and published, disposable
keys. It never connects to a node or broadcasts a transaction. Do not send funds
to its addresses or reuse its keys.

The generated manifest, lockfile, Rust toolchain and evaluator bytes pin the
dependencies needed to reproduce this lesson. Cargo fetches the pinned Sapio and
dependency repositories; no manually cloned sibling libraries are needed.

## Read the contract

[src/contract.rs](src/contract.rs) contains the contract, separately from the
demonstration runner and tests. Its `pay` action accepts a typed `PaymentRequest`
and uses `TemplatePlan` to declare the recipient, change and a 500-satoshi fee.
The policy commits to the recipient, a 5,000-satoshi minimum, exact evaluator
bytes and public oracle root. Every spending path requires this program's
authorization; the tutorial explicitly selects the key path.

An action constructs a proposal; the evaluator decides whether that proposal
satisfies the fixed policy. Proposing another amount does not change the funded
address. The fee reservation is a separate local preparation requirement, checked
again against funding and final transaction weight.

[src/main.rs](src/main.rs) supplies the synthetic 20,000-satoshi funding output,
demo destinations and explicit oracle. The interpreter uses `pay-at-least/v1`;
its parameters encode the minimum and recipient script. Evidence is the
little-endian, four-byte index of the output to check. The runner selects output
zero. The codec label in `evidence.json` describes that format; only evaluation
establishes whether the evidence satisfies the predicate.

## Build and inspect

Run these commands from this generated project. `sapio-cli` must be on your
`PATH`; the repository's [quickstart](https://github.com/sapio-lang/sapio/blob/master/docs/QUICKSTART.md)
shows how to build it. A native C compiler is required. No WASM compiler is
needed for this lesson: the exact evaluator is included in the project.

```sh
cargo test --locked
cargo run --locked -- build demo
cd demo
sapio-cli contract explain --file artifact.json \
  --psbt funded.psbt --assets assets.json
```

The default proposal pays 6,000 satoshis to the recipient and 13,500 to change,
leaving exactly 500 for fees. `build` creates a new directory containing:

| File | Purpose |
| --- | --- |
| `artifact.json` | The compiled contract itself, accepted directly by `contract explain` |
| `funded.psbt` | Base64 PSBT with a synthetic previous transaction |
| `assets.json` | Public program, evidence and signer capabilities |
| `evidence.json` | Exact program requirement and the output-index evidence |
| `oracle.key` | Published demo Xpriv in Sapio's binary key format |
| `pay_at_least.wasm` | The exact evaluator to register for this request |

Inspection validates the artifact and supplied funding. It shows the fixed
program policy and the proposed outputs; the existence of a proposal does not
authorize spending it. Public capability declarations are also not signatures.

## Prepare, sign and complete

Continue inside `demo`. Every output filename below must be new.

```sh
sapio-cli contract spend prepare \
  --artifact artifact.json --psbt funded.psbt --path key \
  --assets assets.json --evidence evidence.json \
  --output intent.json --psbt-output current-0.psbt
sapio-cli contract spend status \
  --artifact artifact.json --intent intent.json --psbt current-0.psbt
sapio-cli contract spend requests \
  --artifact artifact.json --intent intent.json --index 0 --output request.json
sapio-cli signer program \
  --key oracle.key --request request.json --evaluator pay_at_least.wasm \
  --output response.psbt
sapio-cli contract spend apply \
  --artifact artifact.json --intent intent.json --psbt current-0.psbt \
  --response 0=response.psbt --output current-1.psbt
sapio-cli contract spend status \
  --artifact artifact.json --intent intent.json --psbt current-1.psbt
sapio-cli contract spend finalize \
  --artifact artifact.json --intent intent.json --psbt current-1.psbt \
  --transaction --output transaction.hex
```

Before signing, status reports the missing program signature. The signer command
runs the supplied evaluator locally with the explicitly supplied key. Applying
the response verifies it against the original request and adds only its allowed
signature. Finalization fills the selected witness, verifies it and checks the
actual fee before returning transaction bytes.

Each command is a separate process. Keep `artifact.json` and `intent.json`
unchanged and save the latest `current-*.psbt`; those files are enough to resume
between commands. `response.psbt` is a signer's contribution, not a replacement
for your current PSBT. `transaction.hex` contains a locally verified spend of
synthetic funding; it is not a funded transaction ready for broadcast.

Bitcoin sees an ordinary Taproot signature. It does not run this WASM program.
The covenant emulation depends on the oracle enforcing the committed predicate
honestly and remaining available. This contract does not use native CTV.

## Try a rejected payment

From the generated project root, create a separate proposal below the fixed
minimum:

```sh
cargo run --locked -- build rejected --amount 4999
cd rejected
sapio-cli contract spend prepare \
  --artifact artifact.json --psbt funded.psbt --path key \
  --assets assets.json --evidence evidence.json --output intent.json
sapio-cli contract spend requests \
  --artifact artifact.json --intent intent.json --index 0 --output request.json
sapio-cli signer program \
  --key oracle.key --request request.json --evaluator pay_at_least.wasm \
  --output response.psbt
```

The last command must fail because the predicate rejects 4,999 satoshis. It must
not create `response.psbt`. Compilation and preparation can construct this
candidate; they cannot give it signing authority.

[src/tests.rs](src/tests.rs) checks successful completion after serialization,
the rejected payment under the same address, and refusal to turn one extra
satoshi of funding into an unintended fee. Start changing the contract and its
typed request here, and keep those distinctions in your own tests.
