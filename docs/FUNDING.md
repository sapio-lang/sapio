# Contract funding requirements

For new contracts, [transaction plans](TRANSACTION_PLANS.md) offer named input
contributions, exact/remainder allocation and retained local fee rules. The
builder and graph funding representation below remain the common lower layer.
Templates serialize an explicit `funding_constraints` field: `null` for an
ordinary builder template, or ordered requirements from a plan. Recompile older
artifacts rather than dropping the field or inventing a fee policy.


Every compiled contract has an explicit `required_input_amount: bitcoin::Amount`.
It is the minimum amount to allocate to that contract's output and later provide
at its input zero. The JSON field is the required integer
`required_input_amount_sats`. It replaces `amount_range`, whose maximum mixed
minimum funding requirements with allowed upper bounds for address destinations.

## Contract, template and auxiliary amounts

The compiler computes a contract's minimum as:

```text
max(
    ensure_amount(context),
    each committed template's required_input_amount,
    each suggested template's required_input_amount
)
```

Templates are alternatives, so their requirements are combined by maximum,
not addition. Suggestions count because the binder prepares them alongside
committed transactions. `ensure_amount` remains a floor even for a finish-only
contract with no templates. A compilation context supplies a construction
budget; it does not establish that an actual funding output exists.

The fields describe different scopes:

| Rust field | JSON field | Meaning |
| --- | --- | --- |
| `Compiled::required_input_amount` | `required_input_amount_sats` on the contract | Minimum contract-input value, covering its declared floor and all templates |
| `Template::required_input_amount` | `required_input_amount_sats` on a template | Minimum input-zero contribution after declared auxiliary funding |
| `Template::max` | `max_amount_sats` on a template | Aggregate input funding required for outputs and reserved fees |

For builder-produced templates:

```text
aggregate requirement = sum(outputs) + reserved fees
input-zero requirement = max(aggregate requirement - auxiliary contributions, 0)
```

For example, a template paying 1,000 sat with 100 sat of reserved fees and
700 sat of declared auxiliary funding needs 400 sat at input zero and
1,100 sat in total. A parent can fund that child with 400 sat. It must not be
charged the auxiliary contribution again. Binding must still verify that known
inputs supply the aggregate amount.

Use `Builder::add_sequence()` to add an auxiliary input before declaring its
contribution with `add_amount()`. This makes funds available during construction;
it does not select or authenticate a UTXO. The builder checks cumulative funding
and allocation arithmetic, and a single-input template cannot claim auxiliary
funding. Allocate outputs before adding reserved fees. Unallocated construction
budget is not automatically reserved as a fee or a contract minimum.

## Destinations and child outputs

Address and descriptor constructors require an explicit minimum:

```rust
use bitcoin::Amount;
use sapio::contract::Compiled;

// A destination imposing no minimum of its own.
let recipient = Compiled::from_address(address, Amount::ZERO);

// A destination requiring at least 1,000 sat.
let recipient = Compiled::from_address(address, Amount::from_sat(1_000));
```

These are alternative constructor examples. `from_descriptor(descriptor,
minimum)` and `from_script(script, minimum, network)` use the same convention.
`from_op_return` has a zero minimum. Compiling an `XOnlyPublicKey` directly uses
its context's allocated amount as the minimum of the resulting destination.

`Builder::add_output(amount, &contract, metadata)` compiles the child with that
output's allocation and rejects `amount < child.required_input_amount` with
`CompilationError::UnderfundedOutput`. The error includes the output context,
available amount and required amount. This also covers a reused compiled child
and a finish-only child's `ensure_amount` floor. Reusing a `Compiled` object
validates its graph before cloning it; malformed objects return
`CompilationError::InvalidArtifact`. Fresh contract compilation also validates
its completed artifact, catching actions that invalidate a template by mutating
its public fields after using the builder.

For a previously compiled child, the explicit amount is available directly:

```rust
let builder = ctx
    .template()
    .add_output(child.required_input_amount, &child, None)?;
```

The parent still needs enough construction budget for every output and its own
reserved fees. Child requirements may themselves include downstream fees.

## Artifact and binding checks

Public fields and decoded JSON must pass `Compiled::validate()` before their
funding requirements are trusted. Validation checks that each contract's
minimum covers every committed and suggested template's input-zero requirement,
returning `InvalidInputRequirement` if it does not. It also checks that every
parent output covers its child's declared minimum, returning `UnderfundedChild`
with the output index and both amounts. Lowering a child's declared minimum
below its template requirements does not evade the recursive check.

Validation establishes internal consistency. It cannot reconstruct a removed
source-level `ensure_amount` declaration from an arbitrary edited artifact, or
prove that its policies authorize the advertised transaction graph. Preserve
the provenance of artifacts and validate them at consumption boundaries.

Binding performs artifact validation before funding lookups. A known contract
input must match the receiving script and cover the contract minimum, including
when there are no templates. Known inputs are checked independently of unknown
auxiliary inputs: excess auxiliary funding cannot substitute for the declared
input-zero minimum. When every input is known, their checked sum must also cover
the aggregate template requirement. These checks cover descendants using their
actual generated parent transactions.

Unknown inputs remain available for offline construction. A successful bind
with unresolved inputs does not establish sufficient funding. See
[binding contracts](BINDING.md) for UTXO identity checks, signer boundaries and
the returned PSBT data.

The minimum is a construction and binding requirement, not a new Bitcoin Script
amount check. It imposes no upper bound. Excess input funding can become
additional transaction fees when outputs are fixed. The field does not assert
that an output is above dust, that a transaction meets relay policy, that a
script is satisfiable, or that a witness-weight bound exists. Reserved fees and
minimum-feerate checks remain separate; opaque policy backends still have
[unknown satisfaction weight](POLICY_BACKENDS.md#artifacts-binding-and-witnesses).

## Migrating artifacts and callers

An old contract fragment such as:

```json
{"amount_range":{"min_btc":0.0,"max_btc":0.000004}}
```

is replaced, for a contract whose actual minimum is 400 sat, by:

```json
{"required_input_amount_sats":400}
```

Every nested compiled object needs the new field. Template
`required_input_amount_sats` and `max_amount_sats` retain their meanings. There
is no legacy range alias or implicit zero default: an old artifact missing the
new field fails decoding. Recompile artifacts and regenerate consumer schemas.
Do not mechanically convert every old `max_btc` to a minimum: address
constructors formerly used that field for an unrestricted upper bound.

Rust callers replace `compiled.amount_range.max()` with
`compiled.required_input_amount`. Constructor callers replace optional ranges
with the intended `Amount`, usually `Amount::ZERO` for ordinary recipients.
Code constructing `Compiled` directly must supply the field explicitly.

Amounts serialize as integer satoshis without conversion through floating-point
BTC. Rust round trips preserve the full `u64` amount. Consumers must likewise
parse and retain integers exactly; a JSON library that converts all numbers to
floating point can lose precision for sufficiently large integers. This encoding
does not by itself impose Bitcoin's monetary limits.

The [child-funding regressions](../sapio/tests/child_funding.rs) cover both
template maps, auxiliary contributions, finish-only floors, serialization and
rejection before funding or signing effects.
