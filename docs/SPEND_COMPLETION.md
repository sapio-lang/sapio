# Completing a selected spend

`SpendIntent` carries one exact witness selection from preparation through
signing and finalization. Save the immutable intent JSON and the current PSBT
as separate files. An interrupted application can load both with its trusted
compiled artifact and resume without the contract's Rust source or a coordinator
service.

```rust,ignore
use emulator_connect::program::{prepare_spend, completion::SpendIntent};

let intent = SpendIntent::from_prepared(prepare_spend(
    &artifact, selected_path, funded_psbt, input_index, &assets, &evidence,
)?);
let mut current = intent.baseline_psbt().clone();

// Choose the signer explicitly for each original request.
let response = configured_oracle.sign(intent.requests()[request_index].clone())?;
intent.merge_response(&artifact, &mut current, request_index, &response)?;
intent.sign_native(&artifact, &mut current, &native_keys, &secp)?;

let status = intent.status(&artifact, &current)?;
let finalized = intent.finalize(&artifact, &current, &secp)?;
let transaction = finalized.extract_tx()?;
```

The intent retains the selected input, path, exact scriptSig/witness recipe,
original PSBT and independent program requests. Request indexes stay fixed.
Every response is checked against its original request before its sole permitted
signature is merged. Responses can arrive out of order; importing the same
response again is harmless. Replacing the current PSBT with a signer's response
would lose contributions from other signers.

Signing, response import, status and finalization revalidate the artifact,
stored recipe, original requests and current PSBT. After reloading, call
`status(&artifact, &current)` before exporting through the `requests()` accessor.
`request_requirements(&artifact)` returns validated program identities and roots
in the same index order, so each request can be sent to its configured signer.
Deserialization alone grants no authority. Select the artifact
and intent from trusted application state; a matching attacker-supplied pair
does not establish the user's intended payment. Transaction fields, previous
outputs, annex, scripts and existing assets cannot be changed after preparation.
New signature/preimage assets can be added, and sponsor inputs can acquire their
previously absent final fields. If funding or evidence needs to change, prepare
a new intent before requesting signatures.

Native signing is limited to the selected input's ordinary signature slots.
It does not sign Program slots, other branches, or sponsor inputs. Sponsors
must be signed explicitly. Program requests use the existing evaluated-signing
protocol, with no oracle selected from artifact metadata. Key-path requests omit
unrelated leaf proofs and internal-key annotations while the local baseline
preserves them; known-tweak evidence remains the program's explicit proof.

Configure external sponsor wallets to sign without finalizing (`finalize=false`
where supported), so the returned PSBT preserves existing metadata and partial
assets. The wallet must independently restrict which inputs and keys it signs;
the shared `sign_native` API enforces the selected contract's native slots.
Wallet finalization often clears those fields and cannot be imported as ordinary
progress. Alternatively finalize a sponsor before preparing the intent. After
preparation, adding sponsor final fields is allowed only while preserving all
existing data. Native CTV also commits input scriptSigs: the intended finalized
scriptSig view must be known and compatible when selecting that branch.

Finalization fills the retained recipe, verifies all transaction inputs and
checks aggregate input/output amounts. Catalogued templates also enforce their
retained funding and final fee rules before a PSBT is returned. Unselected
inputs can finalize from their already-present assets. A
missing or invalid contribution leaves the caller's current PSBT unchanged.
The finalized PSBT is an output, not a new collection state; resume signature
collection from the saved partial PSBT.
Status reports pending requirements; it does not establish chain maturity,
unspentness, relay policy, or the validity of unverified signatures.

## CLI files

The `contract spend` commands operate locally and do not load network or oracle
configuration. Artifact, intent, asset and evidence files are JSON. PSBT files
contain base64; native key files use the existing binary Xpriv format accepted
by `signer sign`. Commands print to stdout unless `--output` names a new file.
Existing output files are never overwritten.

```sh
sapio-cli contract spend prepare \
  --artifact contract.json --psbt funded.psbt \
  --path 'script:<leaf-hash>' --input 0 \
  --assets assets.json --evidence evidence.json \
  --output intent.json --psbt-output current-0.psbt

sapio-cli contract spend requests \
  --artifact contract.json --intent intent.json
sapio-cli contract spend requests \
  --artifact contract.json --intent intent.json --index 0 \
  --output request-0.json

# Send each exported request to its explicitly configured signer. The returned
# response file contains that signer's base64 PSBT. Responses retain their
# original baselines, regardless of collection order.
sapio-cli contract spend apply \
  --artifact contract.json --intent intent.json --psbt current-0.psbt \
  --response 1=response-1.psbt --output current-1.psbt
sapio-cli contract spend apply \
  --artifact contract.json --intent intent.json --psbt current-1.psbt \
  --response 0=response-0.psbt --output current-2.psbt
sapio-cli contract spend sign-native \
  --artifact contract.json --intent intent.json --psbt current-2.psbt \
  --key native.key --output current-3.psbt
sapio-cli contract spend status \
  --artifact contract.json --intent intent.json --psbt current-3.psbt
sapio-cli contract spend finalize \
  --artifact contract.json --intent intent.json --psbt current-3.psbt \
  --transaction --output transaction.hex
```

Use `--path key` for a key-path spend or `--path descriptor` for a native ECDSA
descriptor. Assets and evidence default to empty if omitted. `requests` exports
an array of `{index, requirement, request}` records; `--index` exports one raw
`ProgramSigningRequest`. `apply` accepts repeated `--response INDEX=FILE` values,
verifies the complete batch in memory, and writes only after every response is
accepted. The validated requirement includes the oracle root needed to choose
the signer; raw requests do not carry that root. Omitting `--psbt` resumes from
the original baseline. Without `--transaction`, `finalize` returns the checked,
finalized PSBT.

To evaluate an exported request locally, select the key and evaluator explicitly:

```sh
sapio-cli signer program --key oracle.key --request request-0.json \
  --evaluator evaluator.wasm --output response-0.psbt
```

This command uses the same evaluated-signing protocol as the Rust API. It never
loads executable code from artifact metadata or chooses a network endpoint. The
[starter walkthrough](../cli/templates/starter/README.md) generates a complete
matching set of files for an allowed payment and a rejected underpayment.

## Examples and recovered proofs

The payment, covenant-fragment and normal eltoo runners all use `SpendIntent`.
Their helper functions only construct explicit evidence, call a configured
example oracle, and sign an explicit sponsor where needed. Shared completion
owns response merging, witness completion and final fee checks.

Eltoo chain recovery has a distinct boundary: it authenticates a published
control block against an observed output and intentionally has no old compiled
artifact. Its `RecoveredUpdate::request` and `finalize_recovered_candidate`
remain explicit lower-level operations, checking the recovered proof and new
target template. Raw protocol counterexamples likewise use the lower-level
signing/finalization APIs to exercise rejection paths.
