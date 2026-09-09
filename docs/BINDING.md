# Binding contracts to funding

`Object::bind_psbt` turns a compiled contract and an input outpoint into a
`Program` of PSBTs. Binding checks the complete artifact and input mappings,
prepares and checks funding throughout the graph, checks every signer response,
and then adds the resulting transactions to the transaction index.

## Funding checks

For each explicit input, the binder obtains the previous transaction and checks
its computed txid against the requested outpoint before selecting its output.
An out-of-range output, an incorrect transaction, or a network/RPC error fails
binding. Only `UnknownTxid` for the requested transaction leaves an explicit
input unresolved. Auxiliary inputs without a mapping remain unresolved and get
distinct placeholder outpoints that avoid explicit mappings and the contract
input.

Known contract inputs must pay the compiled contract's script and provide its
`required_input_amount_sats`. This explicit contract minimum includes
`ensure_amount` and the largest input-zero requirement across committed and
suggested templates. It applies even to a finish-only contract with no
templates, and even when auxiliary inputs are unknown or overfunded.

Each template separately retains `required_input_amount_sats` for input zero
and `max_amount_sats` for the aggregate requirement across all inputs.
Every transaction must use distinct input outpoints, and known input values
must sum without overflow. When all inputs are known, their total must cover
the template's outputs and reserved fees. The artifact rejects a single-input
template that claims external funding. Auxiliary inputs contribute to the
aggregate total; additional funding is permitted and can increase fees when
outputs are fixed.

Before funding lookup, artifact validation checks that contract minima cover
their template requirements and that parent outputs cover their children's
minima. These checks include suggested transactions. Binding then uses generated
parent transactions directly as descendants' funding evidence. The explicit
integer-satoshi field replaces the old `amount_range`; see
[funding requirements and migration](FUNDING.md) for constructor semantics,
auxiliary contribution examples and the limits of these checks.

Every known previous transaction is retained as `non_witness_utxo` in the PSBT.
Native witness outputs also populate `witness_utxo`. This allows a downstream
signer to authenticate the supplied amounts and scripts against the input's
txid. Arbitrary legacy or P2SH auxiliary outputs are not labeled as witness
outputs without evidence of their spending script. See
[BIP-174's input fields](https://github.com/bitcoin/bips/blob/master/bip-0174.mediawiki#specification).

Unresolved inputs support offline construction; a successful bind does not mean
that such a transaction is funded or ready to sign. Transaction identity also
does not establish confirmation or whether an output is still unspent. Those
checks require an appropriate chain/wallet view.

## Signer and index boundaries

The `CTVEmulator` signing contract returns a complete PSBT. A response may add
ECDSA partial signatures, Taproot key signatures, or Taproot script signatures.
It must preserve existing signatures and all other data, including transaction
fields, UTXOs, sighash declarations, scripts, derivations, final scripts, and
unknown/proprietary metadata.

Call `sign_checked` when accepting a custom emulator's response. The HD client
checks the raw response, and a federation checks each participant before
passing the PSBT onward. These checks establish response integrity; finalization
still needs to verify the signatures. They do not authenticate a network peer or
replace a signer's authorization policy.

Invalid funding anywhere in the graph fails before signer calls or
insertion of generated transactions. An invalid signer response also fails
before inserting generated transactions. Index insertion
must acknowledge the locally computed txid; an incorrect acknowledgement or
write error fails binding. Writes are not atomic: an index failure can leave
earlier accepted writes in place. Funding lookups may populate an index
implementation's lookup cache during preparation.

`CachedTxIndex` falls back only on a matching `UnknownTxid`, validates both
transaction contents and insertion acknowledgements, and forwards transactions
with changed witnesses even when their txid is unchanged. Identical
transactions are deduplicated.

## Bound paths and source paths

The root entry retains the compiled root path. Each child entry gets a binding
path formed from its parent's binding path, `@next` or `@suggested`, the template
hash, and `#<output index>`. Reusing a compiled object at multiple outputs
therefore preserves every occurrence, including leaf metadata. Enforced and
suggested transitions remain distinct even if their transaction hashes match.

`SapioStudioObject.source_path` records the original compilation path.
Continuation API paths remain compilation paths. Consumers should use the
program's map keys to identify bound occurrences and `source_path` to relate
them to the compiled contract. Child keys have deliberately changed from the
old source-path keys, which could silently overwrite other outputs.

The CLI's synthetic funding entry uses `<root>/@funding` and has no
`source_path`. It cannot overwrite a contract named `funding` or one of its
transitions. The funding PSBT retains the wallet's supplied metadata and
signatures.

These changes do not solve descendant rebinding for legacy inputs whose
scriptSigs change transaction IDs during finalization, or establish native CTV
enforcement on a particular chain. The supported signing/finalization domain
and remaining release boundaries are recorded in [the fork audit](CTV_FORK_AUDIT.md)
and [the modernization plan](MODERNIZATION.md).
