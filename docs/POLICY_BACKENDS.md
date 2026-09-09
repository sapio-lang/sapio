# Policy backends and raw Tapscript

Sapio accepts custom policy languages through `sapio::policy::PolicyCompiler`.
A backend translates its language into immutable `ScriptPolicy` source; Sapio
then combines that source with action guards, template preconditions and the
transaction covenant. The existing `sapio_base::Clause` alias remains the native
Miniscript policy language.

This follows the custom-language and raw-script direction of
[PR #269, “Sapio w/ Arbitrary Scripts”](https://github.com/sapio-lang/sapio/pull/269).
The current interface retains native policy compilation, contextual metadata,
checked artifacts and funding/binding validation. It is a Rust extension point;
it does not sandbox arbitrary backend code.

## Translating a policy

The interchange types are available from `sapio::policy` or
`sapio_base::policy`:

```rust
pub trait PolicyCompiler {
    fn compile_policy(&self) -> Result<ScriptPolicy, PolicyError>;
}

pub enum ScriptPolicy {
    Miniscript(Clause),
    Script(ScriptFragment),
    And(Vec<ScriptPolicy>),
    Or(Vec<ScriptPolicy>),
}
```

`Clause`, `ScriptPolicy` and `ScriptFragment` implement `PolicyCompiler`.
`Clause` translates to native source without compiling early, so a timelock can
still be combined with its authorization before Miniscript checks the result.
`Result<P, E>` also implements the trait when `P: PolicyCompiler` and
`E: Display`; an error becomes `PolicyError::Backend(error.to_string())`.

Here is a small custom backend. Its arithmetic preserves the signature
condition: `CHECKSIG` must produce one for the final comparison to succeed.
The resulting script lies outside the native Miniscript grammar.

```rust
use bitcoin::blockdata::{opcodes::all, script::Builder};
use bitcoin::XOnlyPublicKey;
use sapio::policy::{PolicyCompiler, PolicyError, ScriptFragment, ScriptPolicy};

pub struct ArithmeticSigner(pub XOnlyPublicKey);

impl PolicyCompiler for ArithmeticSigner {
    fn compile_policy(&self) -> Result<ScriptPolicy, PolicyError> {
        ScriptFragment::new(
            Builder::new()
                .push_slice(&self.0.serialize())
                .push_opcode(all::OP_CHECKSIG)
                .push_opcode(all::OP_1ADD)
                .push_int(2)
                .push_opcode(all::OP_NUMEQUAL)
                .into_script(),
        )
        .map(Into::into)
    }
}

struct Owned {
    owner: XOnlyPublicKey,
}

impl Owned {
    #[sapio::guard(policy, cached)]
    fn signed(self) -> ArithmeticSigner {
        ArithmeticSigner(self.owner)
    }
}

impl sapio::contract::Contract for Owned {
    sapio::declare! {finish, Self::signed}
    sapio::declare! {non updatable}
}
```

Custom guards require the explicit `policy` flag and an explicit return type.
`#[guard(policy)]` receives `(self, Context)`; adding `cached` removes the context
argument. A helper can return a backend, a `ScriptPolicy`, a `ScriptFragment`,
or a `Result` wrapping one of these. Translation errors propagate as
`CompilationError::Policy`; the macro does not unwrap or omit a failed guard.
Ordinary `#[guard]` and `#[guard(cached)]` continue to return native `Clause`s.

Trait interfaces use the matching declarations:

```rust
sapio::decl_guard! { policy signed<ArithmeticSigner> }
sapio::decl_guard! { cached policy signed<ArithmeticSigner> }
```

These illustrate separate fresh and cached declarations; select one for a
given method. As with native guard declarations, the factory is absent until
implemented. A cached guard evaluates its helper and translates its source once
per compilation of that contract. Its `simps = "Some(Self::metadata)"` callback
still runs at every attachment's actual context, including finish guards.

For a transaction-specific precondition, use the template builder inside a
`#[then]` action:

```rust
let builder = ctx.template().add_policy(&ArithmeticSigner(owner))?;
```

`add_policy(&impl PolicyCompiler)` translates immediately and returns a fallible
builder result. `add_guard(impl Into<ScriptPolicy>)` accepts already translated
source, checked fragments, or native clauses directly. Both add a requirement
to that template, in addition to its action guards and covenant. Continuation
templates are suggestions and cannot carry these additional preconditions.

## What a checked fragment guarantees

`ScriptFragment::new(Script)` validates instruction decoding, balanced
`IF`/`NOTIF`/`ELSE`/`ENDIF` control flow and locally balanced alternative-stack
use. Every path starts with an empty alternative stack, cannot read below it,
must merge at equal depth and must finish with it empty. A backend may use
balanced alternative-stack operations inside the fragment.

Validation rejects `OP_SUCCESS` instructions even in untaken branches, illegal
instructions and `OP_CODESEPARATOR`. Code separators are excluded because the
current signing path assumes none. Opcode-like bytes inside pushed data remain
data. Errors identify the offending byte offset. Deserializing a fragment
performs the same checks, and its script is immutable through the public API.

These checks establish composition boundaries. The backend remains responsible
for the meaning of the program and its witness: each fragment must leave its
truth value on top of the main stack, ready for Sapio to append `OP_VERIFY`
and another fragment. It must arrange witness consumption across the complete
branch and satisfy the final clean-stack requirement. Construction does not
prove authorization, satisfiability, main-stack discipline, consensus validity
or a maximum witness size. Tests for a backend should execute complete composed
spends and demonstrate that missing required authorization fails.

## Ordered composition and native runs

`ScriptPolicy::And` requires all children in encounter order.
`ScriptPolicy::Or` permits any child. Both are n-ary: an empty `And` is true and
an empty `Or` has no alternatives. A contract whose complete compilation has no
spending branch is rejected with `CompilationError::EmptyPolicy`.

Nested IR alternatives expand recursively. Conjunction takes their Cartesian
product while preserving each branch's operand order. For example:

```text
And([Or([A, B]), Or([C, D])])
    -> [A VERIFY C, A VERIFY D, B VERIFY C, B VERIFY D]
```

Adjacent native operands are combined and compiled as one Miniscript run.
Consecutive runs and raw fragments are joined with `OP_VERIFY`. Raw fragment
order and multiplicity are preserved, including repeated inscriptions; the
compiler never moves a native predicate across raw code to make it compile.
Identical complete leaf scripts may share one leaf in the final tree.

Each native run must independently satisfy Miniscript's compiler checks.
Adjacent native key and timelock clauses can form a safe run. A raw signature
predicate followed by a separate native timelock can fail native safety checks:
the compiler cannot infer that the raw predicate supplies the missing native
authorization. Keep native authorization adjacent, or have the custom backend
emit a complete raw predicate for the combined condition.

Use `ScriptPolicy::Miniscript(clause)` when the whole policy belongs to the
native language. Native `Clause::And` and `Clause::Or` require two operands, and
native thresholds require a valid threshold. Source validation runs before
simplification, including for malformed timelocks and inscription fields in
branches that would otherwise disappear. Repeated keys in separate native
alternatives remain valid; each eventual native script receives its own checks.

The IR path treats emitted scripts as opaque even if their bytes happen to
match Miniscript. Wrapping native policies in general IR nodes does not promise
native witness analysis. Nor does the compiler promote a key found inside raw
code to an unconditional Taproot key-path spend. Only an existing standalone
native key alternative is eligible for that optimization.

## Compilation budgets

The following are local work limits enforced during policy lowering:

| Resource | Per policy lowering |
| --- | ---: |
| Source depth, root at zero | 128 |
| Source nodes, including nested native clauses | 65,536 |
| Expanded alternatives | 1,024 |
| Operand slots across expanded alternatives | 65,536 |
| Aggregate source raw-script and inscription payload bytes | 16 MiB |
| Payload bytes repeated across expanded alternatives | 16 MiB |
| Cumulative encoded branch-script bytes, including inserted verification | 16 MiB |

Expansion costs are checked before materializing the alternative product.
Unreachable source is still checked. Native inscription bodies and metadata can
span multiple 520-byte pushes; 520 bytes is not a total inscription-body limit.
There is no separate 64 KiB raw-leaf limit.

Each contract compilation additionally admits at most **1,024 branches** and
**16 MiB of aggregate encoded scripts** across its actions and finish guards.
These counts precede final leaf deduplication. Each compiled child contract has
its own budget. Exceeding a bound returns `CompilationError::PolicyLimit` with
the resource and limit.

These bounds do not cap arbitrary Rust backend execution, all allocations,
native compiler CPU time or an entire contract graph. They are separate from
consensus rules, witness-weight estimates and serialized artifact/transport
limits. In particular, JSON's hex encoding expands script bytes; passing the
script-byte limit does not guarantee fitting a WASM ABI message. Direct
`RawTaproot` construction and deserialization enforce tree and leaf-count checks,
not the compiler's complete source/expansion byte budget.

## Artifacts, binding and witnesses

Pure native output retains its native descriptor representation. Output with
opaque branches uses `SupportedDescriptors::Taproot(RawTaproot)`. Its serialized
spending data contains only the internal key and a depth-first list of
`(depth, script)` leaves; depths start at zero and scripts are hex strings.
Output keys, Merkle roots and control blocks are derived again on decoding.

`RawTaproot` checks every script, limits explicit leaves to 1,024 and rejects
incomplete or invalid trees. Its leaves use the current TapScript version.
The compiler builds a deterministic equal-weight tree of distinct complete
scripts. An explicitly constructed raw artifact may retain duplicate scripts
at different tree positions and their control-block proofs. An empty leaf list
explicitly describes a key-only raw object; it is distinct from compiling a
contract with no spending branches.

Call `Compiled::validate` before consuming a publicly constructed or decoded
artifact. Binding also validates the artifact and the supplied funding data.
In particular, the descriptor or raw tree must commit to the advertised output
script. Validation does not establish a custom backend's authorization.
Binding exports the internal key, Merkle root and every leaf/control-block proof
into the PSBT, preserving existing template and funding checks.

Any opaque branch makes the contract's maximum satisfaction weight unknown.
If a committed template requests a minimum feerate, compilation fails with
`UnknownSatisfactionWeight`. There is no guessed weight or backend-declared
weight accepted as proof. Backends need their own witness construction and fee
planning when using opaque scripts.

Signers may add partial signatures to these PSBTs. The existing Miniscript
finalizer can still finalize through a recognized alternative or key path. If
finalization fails and a TapScript leaf cannot be parsed as Miniscript, the
`NotFinished` response identifies its input and leaf hash and reports that
spending that leaf requires an external satisfier. The returned PSBT retains the
raw spending data and partial signatures for that unfinished input; successful
native inputs can retain their finalization progress. Structurally invalid PSBTs
remain errors.
An external finalizer must construct the custom witness and validate execution;
the native finalizer does not certify arbitrary script programs.

## Updating artifact consumers

Guard metadata now uses policy records so that native and custom sources have
the same representation. The old clause-keyed object:

```json
{"simps_for_guards":{"older(10)":{"-17":[{"label":"mature"}]}}}
```

becomes:

```json
{"simps_for_guards":[
  {"policy":{"Miniscript":"older(10)"},
   "protocols":{"-17":[{"label":"mature"}]}}
]}
```

Records use deterministic policy order. Within each protocol, equal JSON values
are deduplicated and distinct attachment values retain encounter order. This
metadata describes the source guard; the committed output determines spending.

Template `additional_preconditions` also stores tagged `ScriptPolicy` values:
`["older(10)"]` becomes `[{"Miniscript":"older(10)"}]`. Raw source uses
`{"Script":"hex"}`, and `And`/`Or` contain arrays of child policy values.
Consumers must also handle the new `Taproot` variant of `known_descriptor`.

Recompile old artifacts and regenerate consumer schemas. There is no legacy
field alias or read fallback that guesses a custom policy from old data.
Direct Rust users of `Template::guards` must convert native clauses with
`.into()`; builder calls to `add_guard(native_clause)` continue to work.

See [contract action semantics](LANGUAGE_SEMANTICS.md), the
[policy API](../sapio-base/src/policy.rs),
[backend integration tests](../sapio/tests/custom_policy.rs) and
[external-finalization tests](../sapio-psbt/tests/custom_finalization.rs) for the
corresponding implementation and regression cases.

## Node validation

The native backend regression and WASM catalog demonstrate compilation, artifact
validation and binding. A separate oracle executes complete custom witnesses
against Bitcoin Core 31.1 on an isolated regtest node:

```sh
cargo build --locked -p sapio --example custom_policy_vectors
python3 contrib/check_custom_policy.py \
  /path/to/bitcoin-31.1/bin/bitcoind \
  /path/to/bitcoin-31.1/bin/bitcoin-cli \
  target/debug/examples/custom_policy_vectors
```

The driver creates temporary node data and a temporary wallet, mines funding,
and tests spends of actual outputs with `testmempoolaccept`. It does not access
an existing node or wallet. It requires permission to bind loopback sockets.

Core accepts two valid transactions: a custom arithmetic signature predicate,
and that predicate composed with a native action signature, a custom template
signature and an emulator-provided key clause. Ten invalid variants reach script
verification and fail: missing each required signature, a wrong owner signature,
and outputs changed after signing. One changed-output case has freshly generated
owner/action/template signatures but no covenant-signer signature.

The fixture supplies a fixed emulator key and constructs its signatures
explicitly; it tests preservation of the emulator's authorization clause, not
an emulator service's transaction approval rule. Stock Core does not enforce
native CTV. Native CTV compilation and transaction-hash tests remain separate
from these ordinary Taproot execution checks. The Ubuntu native CI job runs the
same oracle with a checksum-pinned Core binary.
