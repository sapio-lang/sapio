# Typed module composition

A module exports its input and output JSON Schemas through the existing API:

```json
{"arguments": "CreateArgs<T> schema", "returns": "R schema"}
```

`arguments` includes the compilation `context` and the module's own
`arguments` payload. Editors expose the payload fields as input sockets.
Both schemas are independently rooted Draft 7 documents, including their
own definitions. Editors must preserve those definitions when extracting
a field schema.

## Values and interfaces

`x-sapio-type` names a semantic value type. A shared Rust DTO can declare it
without changing its serialized representation:

```rust,ignore
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(title = "Authorization", extend("x-sapio-type" = "sapio.authorization"))]
pub struct KeySet {
    pub threshold: u8,
    #[schemars(schema_with = "sapio_base::schema::public_keys")]
    pub keys: Vec<XOnlyPublicKey>,
}
```

Shared schema helpers in `sapio_base::schema` identify public keys, addresses,
extended public keys, integer satoshi amounts, and relative block counts.
Semantic identities distinguish values with identical JSON representations.
They do not replace JSON Schema constraints, network checks, threshold
validation, or cross-field invariants enforced by the consuming module.

`SapioHostAPI<T, R>` exports `x-sapio-module` with the expected `arguments`
and `returns` schema nodes. `arguments` describes the full `CreateArgs<T>`
envelope. Both nodes use local references in the containing top-level API
schema root, sharing its definitions; they are not independent schema roots.
This keeps recursive callable interfaces finite. Arguments use the
deserialization contract and returns use the serialization contract, even
when the handle is itself nested inside an input or a returned value.
Editors extracting a field must retain its containing root and rebase local
references inside callable annotations along with ordinary schema references.
This is a callable implementation socket, not a socket for its returned value.
The serialized reference still contains only `which_plugin`; the expected
signature comes from the consumer's Rust declaration. `ContractModule<T>` and
`ClauseModule<T>` are the existing concrete conveniences for these handles.

The compiled artifact schema exports `x-sapio-role: "contract"` and
`x-sapio-type: "sapio.compiled-contract"`. An editor uses the explicit role
to distinguish a contract-building node from a value-producing module.

## Host checks

Typed calls pass their expected signature to
`sapio_v1_wasm_plugin_create_contract_typed`. Before running the child module's
creation function, the host reads its live API, validates both schema pairs
offline, and compares the signatures. It also validates actual input values
and successful output values against the live schemas.

Interface matching is exact, with local references resolved and documentation
annotations and definition names ignored. Recursive Rust types are supported.
Validation constraints and semantic identities must match; module authors
use explicit adapters for different interfaces. External references and
nested reference-resource scopes are unsupported in typed signatures.

Matching declarations establish API compatibility. They do not certify the
behavior of a module or the security of a compiled custody policy. A consumer
must still select a trusted implementation and enforce its own invariants.

## Discovery cache

The CLI stores disposable JSON metadata under `metadata/<source hash>.json`
beside its authenticated WASM source cache. The cache includes the module's
name, input/output schemas, and logo. Schema validation occurs before cache
publication; the format version, source identifier, and checksum are checked
when reading. Warm discovery authenticates source bytes without compiling or
instantiating WASM. Missing, stale, or corrupt metadata is reconstructed through
normal metered module loading.

This cache accelerates `list`, `api`, `info`, `logo`, and `load`. Its checksum
detects accidental corruption; it is not a signature or an attestation by a
trusted publisher. Execution obtains schemas directly from the running module
and never uses cached metadata to authorize a call. Native executable caches
are never loaded.
