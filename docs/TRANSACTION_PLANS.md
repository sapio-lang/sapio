# Transaction plans and spend preparation

Sapio separates three questions:

1. **Construction:** which exact transaction does this action propose?
2. **Enforcement:** which predicates authorize spending the output?
3. **Satisfaction:** which signatures, preimages, program evidence and transaction
   fields does one complete spending branch need?

`TemplatePlan`, typed `Action` handles and the spend planner answer these questions
without making one stage silently perform another. The executable
[payment](../sapio/examples/payment.rs),
[template authorization](../sapio-contrib/src/contracts/template_authorization.rs)
and [eltoo](../sapio-contrib/src/contracts/eltoo/mod.rs) contracts demonstrate the
frontend. Signing, disposable keys and adversarial scenarios live in the
[integration runners](../integration_tests/src/).

## Declare a transaction

```rust,ignore
let mut plan = ctx.template_plan();
plan.output("recipient", OutputAmount::Exact(payment), &recipient)?;
plan.output("change", OutputAmount::Remainder, &change)?;
plan.reserve_fees(fee);
let template = plan.finish()?;
```

`finish` solves a finite allocation problem with checked integer arithmetic. It
first resolves output amounts and checks lock and ordinal constraints, then
compiles each child once with its exact allocation. It does not discover child
requirements by repeatedly compiling them or search for a transaction behind the
author's back. A child that needs more funds produces an error naming its output.

The contract input is named `contract` and remains input zero. `input(name,
minimum)` adds an auxiliary input in declaration order; its minimum contribution
adds to the planning budget. Outputs also retain declaration order, including a
remainder declared before an exact payment. At most one output receives the
remainder. Names are unique within each input/output group. References resolve
by name within the plan, so helpers can refer to a role without guessing an
index; they are not globally unique identities.

Repeated fee reservations take the maximum, allowing independent helpers to
require a minimum fee without double-charging it. Repeated same-unit timelocks
also take the maximum. Mixing height and time requirements is an explicit
conflict. A rejected declaration leaves the previous requirement intact.

`Builder` remains the lower-level API for procedural construction. A plan freezes
into the same `Template` consumed by the compiler; it does not create a second
transaction or covenant representation.

## Allocate every satoshi and retain local funding rules

By default, all planned funds must go to outputs or reserved fees. An unallocated
balance is an error. A remainder output allocates it to a declared destination.
`Surplus::Fees { maximum }` explicitly allows the balance to become fees, subject
to an absolute ceiling. The ceiling also applies to additional actual funding
supplied after compilation. With the default `Surplus::Reject`, actual fees must
remain exactly the reservation.

For a sponsor that pays the final fee without changing the committed outputs:

```rust,ignore
let sponsor = plan.input("sponsor", Amount::ZERO)?;
plan.surplus(Surplus::Fees { maximum: maximum_sponsor_fee });
```

Zero is a legitimate declared minimum. A positive minimum must be met by that
specific input: sufficient money elsewhere cannot conceal an underfunded sponsor.
The plan retains ordered names, contributions, fee cap and an optional typed
`bitcoin::FeeRate` in `Template.funding_constraints`.

These are **local preparation rules**. Input values and a fee ceiling are not
additional covenant predicates. Another party bypassing this API is still bound
only by the committed Bitcoin spending policy. An artifact with modified policy
can change those local rules without changing its address; applications must
obtain their intended artifact from a trusted source.

Artifact validation checks the declarations' internal consistency. A fee cap
that cannot meet the requested rate even at unsigned transaction weight is
rejected immediately. The same transaction appearing in both catalogs must
retain identical binding data, including local funding rules; authorization
guards can remain different alternatives. Graph binding
checks supplied input amounts; matching-template program request preparation and
whole-branch preparation check them again against supplied PSBT previous outputs.
Missing previous outputs remain unknown. Witness-only previous outputs are caller
assertions about the amounts/scripts that a signature will commit to; they do not
prove chain existence, confirmation or unspentness.

`require_feerate` retains a residual obligation. `check_funded_psbt` reports it as
pending until all funding amounts and final spending fields are supplied. After
normal signature finalization, call this method again to check the measured
transaction weight before extraction. It measures the supplied final witness;
it does not establish that a witness is valid. The older builder's
`min_feerate_sats_vbyte` annotation is not a substitute for this typed check.

## Preserve ordinal placement

`require_ordinal(&output, ordinal, offset)` requires a tracked ordinal at an exact
zero-based offset in the declared output. It checks placement after resolving
amounts and before compiling children. It never reorders outputs, moves a
protected satoshi into fees, or invents ranges for unknown inputs.

Tracked input ranges must describe the entire contract allocation. When an
auxiliary input has unknown ordinal ranges, the tracked prefix must be allocated
before the unknown contribution can be used. An output spanning that boundary is
rejected with a named diagnostic. Arbitrary ordinal routing or output-order search
is not part of this resolver.

## Give each action its own request type

```rust,ignore
#[sapio::contract]
impl Payment {
    #[policy]
    fn authorize(&self) -> EmulatedProgram { self.program.clone() }

    #[action(suggested, guarded_by(Self::authorize))]
    pub fn pay(&self, ctx: Context, request: PaymentRequest)
        -> Result<Template, CompilationError>
    {
        // Declare and finish the transaction plan here.
    }
}
```

Methods remain ordinary Rust methods. `Payment::pay_action()` is a typed handle
for invocation, schema publication and request encoding. `handle.request(&root,
&request)` constructs the effect database for exactly that action;
`handle.requests(&root, &requests)` labels several candidates deterministically.
The compiler erases each action's request type only at its dispatch boundary.
There is no contract-wide request enum or coercion from unrelated action values.

Committed actions add the covenant wrapper; suggested actions publish candidates
under an independently fixed spending policy. A unit request is an explicit JSON
`null` entry. No entry means no request. Default candidates use a separate
`defaults = Self::proposals` callback. A no-argument committed action supplies its
normal default transition; a no-argument suggested action opts in with `default`.
Unknown raw effect paths are not globally diagnosed as unused: typed handles
avoid spelling those paths manually, but they are not a global consumption log.

## Plan and prepare a complete spending branch

`emulator_connect::program::plan_spends` takes a validated artifact, an optional
PSBT/input index, and explicit `SpendAssets`. It reports independent key paths,
script leaves and supported descriptor spends. Within each leaf it chooses one
complete nonmalleable native satisfaction, preferring the declared assets. It
reports missing signatures/preimages, native transaction lock checks, exact
program requirements and evidence codecs, descriptor proofs and witness bounds.
It does not enumerate every internal Boolean combination.

A program capability identifies the complete evaluator/program/parameters,
public root and signature path. An ordinary Schnorr-key capability cannot stand
in for that program. An existing signature is reported as present but unverified;
preimages are checked against their hashes. Chain maturity remains unknown
without chain data. Unsupported raw scripts remain explicitly unsupported.

`prepare_spend` requires the caller to select one branch and provide exact
`ProgramEvidenceAdapter` values for its unsigned program slots. The portable
`ProgramEvidence` implementation carries a full requirement, an application
codec and bounded bytes. Preparation supplies descriptor proofs and returns the
selected branch's unsigned program requests. Native signatures or preimages may
still be pending. Requests for other branches are never produced implicitly.

Neither a codec string nor SIMP metadata grants authority. The caller configures
evidence adapters and signers explicitly. Preparation performs no network access,
oracle invocation or signing. Evaluating and signing a program request, merging
its response, and normal Bitcoin signature finalization remain explicit steps.
