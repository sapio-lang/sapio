# Bitcoin 0.32 migration

Sapio uses Bitcoin **0.32.102**, its upstream **secp256k1 0.29.1** dependency,
and Miniscript **13.1.0**. Native applications and compiler plugins use the
same pinned sources. The package names and Cargo features are upstream names.

## Fork boundaries

The Bitcoin branch starts at the upstream `bitcoin-0.32.102` release. Its two
additional changes enforce canonical Taproot signature encodings and complete
consumption by the PSBT slice decoder. The streaming reader remains available.
Both changes reject malformed data before decoding can hide the original bytes.

The Miniscript branch starts at upstream `miniscript-13.1.0`. It carries CTV
commitments, inscription envelopes and their policy/compiler/interpreter support,
plus transaction-wide PSBT finalization and sighash checks. Its CTV satisfier
fails closed without a supplied commitment. Interpreter users call
`with_tx_template` after the standard five-argument `from_txdata` constructor.

External Cargo workspaces must copy the complete `[patch.crates-io]` table
from Sapio's root manifest. Cargo does not inherit dependency patches through
library dependencies. See [development instructions](DEVELOPMENT.md).

## Application API changes

- Owned scripts are `ScriptBuf`; borrowed script arguments use `&Script`.
- Transaction versions, sequences, locktimes and output amounts use upstream
  types. Explicit consensus conversions preserve the encoded transaction bytes.
- Deserialized addresses are unchecked. Contract compilation validates their
  network before accepting them as checked addresses.
- `Psbt::serialize` and `Psbt::deserialize` replace consensus codec calls for
  PSBT messages. Final transaction export uses checked `extract_tx`, retaining
  the PSBT when extraction fails. Computing a signing commitment does not
  require fee metadata and includes any finalized scriptSigs.
- Concrete policies use `Arc` children, `Thresh(Threshold)` and typed timelocks.
  Dynamic invalid thresholds return errors. Converting a Sapio transaction
  timelock into a policy guard is fallible: valid transaction fields can lie
  outside Miniscript's guard domain.
- Sapio owns `sapio_base::amount::CoinAmount` and its tagged `Sats`/`Btc` input
  format. The `sapio-jsonschema` fork provides opt-in Bitcoin and Miniscript
  `JsonSchema` implementations, so module boundaries use `Clause` directly
  with its upstream policy string format.
- Human-readable outpoints use `"txid:vout"`. Core fixture drivers and contract
  schemas follow upstream serialization.
- Host RPC uses upstream `bitcoincore-rpc 0.19`. The synchronous transaction
  index calls it directly; asynchronous CLI handlers run requests on blocking
  workers. CLI configuration also supports Testnet4.

## Verification

The migration retains the complete CTV and BIP446 vector corpora, inscription
acceptance/rejection cases, signing and PSBT mutation tests, and the eltoo
stale-state override/settlement scenarios. Run the commands in
[DEVELOPMENT.md](DEVELOPMENT.md) for native, plugin, WASM and Core checks.

Evaluator WASM formats and distributed evaluator bytes remain unchanged. The
host crypto tests verify the upgraded secp256k1 implementation through the same
ABI, including the existing v2 internal-key and annex context.
