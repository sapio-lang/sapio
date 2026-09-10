# Covenant lowering and enforcement assumptions

Sapio compiles explicit covenant predicates using public, serialized inputs.
This lowering does not consult host callbacks, DNS or reachable oracles. Signer
connections belong to binding and signing. The resulting artifact records the
inputs and predicates needed to check later reuse and signer compatibility.

## Explicit predicates and pure lowering

`sapio_base::Ctv(hash)` represents a BIP-119 template predicate.
`Emulatable(Ctv(hash))` explicitly permits that predicate to use the compilation
context's `LoweringPlan`. Its context-free `PolicyCompiler` implementation
produces `ScriptPolicy::Emulatable`; it does not derive a key or contact a host.
Ordinary `Clause::TxTemplate` predicates and raw scripts retain their native
meaning and are never rewritten by the plan.

For a contract with a `predicate: Ctv` field, import `Ctv` and `Emulatable`
from `sapio_base`, and `guard` from `sapio`, then declare a policy guard:

```rust
impl MyContract {
    #[guard(policy, cached)]
    fn covenant(self) -> Emulatable<Ctv> {
        Emulatable(self.predicate)
    }
}
```

Attach it with `declare! {finish, Self::covenant}` or an action's
`guarded_by = "[Self::covenant]"`. The cached source contains the predicate;
each compilation resolves it using that compilation's public plan.

The two public lowering choices are:

| `LoweringPlan` | Result for an explicit CTV wrapper |
| --- | --- |
| `Native` | `Clause::TxTemplate(hash)` |
| `CtvEmulation { signers, threshold }` | Locally derived signer keys using the existing CTV-specific BIP32 path |

A single signer with threshold one produces a key clause. Multiple signers
produce a threshold clause in the configured order. Roots contain only extended
public keys; endpoints, sockets, timeout settings and secret keys are absent.
The plan rejects zero or excessive thresholds, duplicate derivation identities,
and roots too deep to derive the nine-child CTV path. Derivation identity uses
the compressed public key and chain code, so changing an xpub's network or
parent metadata cannot manufacture an independent signer.

`Context` carries these public inputs, and nested compilation inherits them.
Compilation validates the plan even if the contract uses no CTV wrapper.
JSON module arguments require an explicit `context.lowering`, for example:

```json
{
  "context": {
    "network": "Regtest",
    "amount": 10000,
    "lowering": "Native"
  },
  "arguments": {}
}
```

Emulation uses the externally tagged shape
`{"CtvEmulation":{"signers":["<extended public key>"],"threshold":1}}`.
Changing these compilation inputs can change the script. Changing runtime
signer addresses cannot. CLI compilation does not resolve signers, and the
WASM host exposes no signer-selection or PSBT-signing callback. There is no
new module-information ABI or claim of arbitrary Rust callback purity; contract
authors remain responsible for avoiding other ambient state in their code.

## Actions and artifact requirements

Both transaction actions and continuations use the shared action representation
with `TemplateKind::Covenant` or `TemplateKind::Suggested`:

| Action kind | Returned templates | Policy behavior |
| --- | --- | --- |
| `Covenant` (`#[then]`) | Committed transitions | Add an explicit `Emulatable(Ctv(template_hash))` predicate alongside action and template guards |
| `Suggested` (`#[continuation]`) | Transaction suggestions | Retain the continuation's declared guards; reject additional guards on returned templates |

The compiler resolves wrappers wherever they occur, including finish guards,
continuation guards and custom policy sources. Custom raw predicates retain
[their composition rules](POLICY_BACKENDS.md). This does not require every
backend to expose a Miniscript AST.

Each compiled object records:

```text
covenant_requirements {
    lowering: LoweringPlan,
    predicates: BTreeSet<Ctv>
}
```

This set records the wrappers resolved for that object, including predicates
that do not correspond to a returned transaction. Descendants retain their own
requirements. Structural validation checks the recorded plan and requires each
committed template's CTV predicate to appear in the object's requirements.
Suggestions do not create an automatic CTV predicate, but their guards or
children can have explicit requirements.

For each committed template, feasibility checks retain the original wrapped
CTV predicate and compare its hash with the fixed transaction. Lowering a
wrapper to a signer key cannot turn an incompatible template guard into a
possible transition. This check also covers known native constraints; opaque
raw predicates retain their backend's responsibility for feasibility.

The old `returned_txtmpls_modify_guards` Boolean and
`extract_clause_from_txtmpl` callback are replaced by `template_kind`; there
are no compatibility aliases. Module inputs and object artifacts now require
the explicit lowering data. Recompile older artifacts from trusted source
rather than guessing the missing policy inputs.

## Reuse and signer compatibility

`Object::validate_for_lowering` checks artifact consistency and compares the
policies derived from every object's recorded plan with those derived from a
new public plan. Compiled-object reuse calls this pure check. It traverses both
committed and suggested descendants and covers explicit finish predicates as
well as generated template checks.

`Object::validate_for_emulator` performs the corresponding runtime compatibility
check using the signer's advertised `get_signer_for(hash)` policy. Binding
checks this before funding lookup, signing or index insertion; the CLI checks
before wallet activity. Equality is exact `Clause` equality, not a proof of
semantic equivalence between different policy representations. See
[binding checks](BINDING.md) for funding and signature-response integrity.

These checks concern the predicates already recorded in the artifact. They do
not identify operators, authenticate an endpoint's custody of a secret key,
promise a future continuation's policy or establish service availability. A
threshold is an authorization condition, not a promise that enough peers will
respond.

Artifacts still require trusted provenance. Public fields or serialized data
can be changed together. Validation does not prove that arbitrary advertised
requirements match the committed descriptor or authorize exactly the declared
transaction graph. The actual spending script remains the authority.

## Runtime CLI assumptions

Runtime `CovenantConfig` is separate from `context.lowering`. It selects the
binding/signing service and the operator's native CTV assumption:

| `mode` | Meaning |
| --- | --- |
| `native_ctv_research` | Bind using native template checks on a chain the operator assumes enforces CTV |
| `signer_emulation` | Use the configured peers and signer threshold |
| `signer_emulation_with_native_ctv_research` | Use signer emulation and also assume the chain enforces direct native CTV instructions |

The runtime configuration uses a `covenant` object with the `mode` tag. The two
signer modes supply `emulators`, `threshold` and optional
`request_timeout_secs`. Invalid configuration, resolution failures and signing
failures never select another backend as a fallback. Selecting native research
does not override an artifact's recorded signer policy: the compatibility check
still rejects a mismatch.

Sapio does not infer native activation from an address network, `regtest`, RPC
connectivity or successful compilation. A deployment using either native
research mode must establish its node's intended opcode semantics separately.

## Direct native instructions

`Object::requires_native_ctv` inspects known spending scripts throughout the
stored graph. It scans every Miniscript and raw Taproot leaf and known
witness/redeem script for the `OP_NOP4` instruction used for CTV. Pushed bytes,
including inscription bodies and hash values, do not count as instructions.

The scan conservatively includes alternative leaves and untaken branches.
Ordinary `signer_emulation` rejects a positive result before wallet funding.
The combined mode permits signer-lowered wrappers beside direct native guards,
with an explicit native activation assumption. It can be needed even when a
particular spend will not take the native branch.

Address-only destinations reveal no spending script to inspect. A negative
result means no CTV instruction was found in the available scripts; it does
not establish the behavior of hidden scripts. This native-instruction rejection
is an additional CLI policy, distinct from library policy-equality checks.

## Generic encumbrance programs: design boundary

`EmulatedProgram` supports an exact evaluator identity, program bytes, preset
parameters and public oracle root through the existing `PolicyCompiler` API.
It derives an ordinary signature-key clause without consulting a runtime.
The [program-emulation guide](PROGRAM_EMULATION.md) specifies its commitment,
separate derivation namespace, versioned signing protocol and executable
non-CTV payment example. `Emulatable<P>` itself still implements only CTV.

Rubin's oracle-assisted model derives a public key from a complete encumbrance
instance, including preset parameters, and signs conditionally on a predicate
of a transaction and witness. Its BitVM accountability construction is a
separate mechanism. This motivates separating public lowering from runtime
signing without claiming that Sapio implements that full system.
[Un-FE'd Covenants, sections 2.1–2.2](https://rubin.io/public/pdfs/unfedcovenants.pdf)

The version-one commitment covers every instance byte with tagged hashing and
explicit lengths. Generic derivation adds a fixed child before the existing
nine-child hash encoding, leaving CTV derivation unchanged. These paths are
distinct under the same configured root; arbitrary related xpubs do not
establish independent signer custody.

`ProgramOracle` executes inline WASM for the reserved zero evaluator identity
and registers other WASM interpreters by their exact module hash. All evaluate
in fresh instances under the [version-one ABI and resource
limits](WASM_EVALUATORS.md). A request carries
the complete instance, selected input and Taproot path, PSBT and auxiliary
witness. Unknown evaluators, invalid signing contexts and rejected predicates
fail without producing a signature. `ProgramClient` verifies the derived
signature and permits only its exact PSBT insertion. The existing PSBT-only CTV
service remains separate and cannot process these requests.

Evaluators receive only the fields committed by the supported Taproot
`SIGHASH_ALL` signature. The view excludes scriptSigs, witness contents,
transaction weight, current txid and arbitrary PSBT metadata. The signer uses
no annex, requires a non-finalized selected input, and accepts only supported
key paths or authenticated TapScript leaves without code separators. Auxiliary
witness is an existential proof input; it does not promise publication or
commit those bytes to the transaction.
[BIP341 signature message](https://github.com/bitcoin/bips/blob/master/bip-0341.mediawiki#common-signature-message)

Generic program assumptions are not inferred from a key or added to
`covenant_requirements`. Applications must retain the complete `EmulatedProgram`
and dispatch requests explicitly; the example stores it in object metadata.
CTV lowering plans, binder compatibility checks and CLI covenant modes retain
their existing CTV scope.

Evaluators must implement their declared semantics, avoid ambient mutable state
and bound their synchronous work. The library does not enforce Rust callback
purity or provide an execution sandbox. An interpreter for uploaded programs,
automatic generic artifact dispatch, proof publication, BitVM disputes and bonds
remain future work. Tests establish instance identity, signature boundaries and
accepted/rejected spends, not penalties for a dishonest signer.
