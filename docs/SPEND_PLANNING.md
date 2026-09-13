# Inspecting and preparing a spending branch

`emulator_connect::program::plan_spends` inspects a validated artifact without
signing, evaluating a program, or choosing an oracle. Supply an optional PSBT
and input index plus a `SpendAssets` inventory of explicit capabilities:

```rust,ignore
use emulator_connect::program::{plan_spends, SpendAssets};

let report = plan_spends(&artifact, Some((&psbt, input_index)), &SpendAssets::default())?;
```

The report lists every Taproot key path and distinct script leaf, or the native
ECDSA descriptor's selected satisfaction. Each supported leaf uses Miniscript
to choose one complete non-malleable witness with the supplied assets. When
assets are missing, it describes a hypothetical completion. It does not
enumerate every possible choice inside a threshold or nested alternative.
The ordered witness template and required keys/preimages identify the selected
completion; it does not flatten alternatives into a list of mandatory keys.

An asset can be missing, available from an explicit capability, present but
unverified, or present and checked. PSBT signatures remain unverified in this
report. Native preimages must have the expected 32-byte length and digest before
they count as present and checked. Ordinary Schnorr-key capabilities cannot
stand in for a retained program requirement.

For timelocks, transaction compatibility checks version, sequence enablement,
units and values. Chain maturity stays unknown because this API has neither a
chain clock nor coin confirmation ages. An opaque script has an unsupported
status and no witness-weight bound. Native scripts with unsupported satisfaction
structure or mixed timelocks likewise cannot produce a successful plan.
Native CTV requirements retain the exact selected template hash. With a PSBT,
the check includes every finalized input scriptSig, as required by BIP119.
Adding or changing a scriptSig later requires recomputing that check.
Without a PSBT, the planner separately considers each distinct commitment and
a CTV-free satisfaction; accepting conflicting hashes together would invent an
impossible conjunction. These candidates remain internal, with one selected
completion per path. Equal-cost candidates prefer CTV-free spending, then
bytewise hash order. Native planning has a shared budget of 65,536 work units
across all paths: each discovery or solver pass charges its policy's node count.
This bounds planning input work, not internal Miniscript traversal counts.
Exhaustion produces an
unsupported branch with no weight bound, even if a partial search found a
candidate. Typed Program-based CTV retains its separate evidence requirements.

Funding checks validate PSBT shape, supplied previous-transaction identities,
duplicate inputs and conflicting previous-output data. Known funding at the
selected input must match the artifact's receiving script and minimum amount.
Witness-only previous outputs remain caller assertions; these checks do not
prove chain inclusion or unspentness. Missing previous outputs are reported.
An exact catalogued template at input zero also supplies its retained local
funding and fee constraints. Arbitrary uncatalogued program transactions do not
acquire inferred local rules.

## Explicit program evidence

`SpendAssets.programs` contains `ProgramCapability` declarations. Each binds
the complete `ProgramRequirement`—evaluator, program, parameters, oracle root
and signature path—to an application-defined codec and separately declared
evidence/signing availability. SIMP labels and endpoint hints cannot supply
this authority. Declaring an adapter never establishes evaluator success.

`prepare_spend` accepts an explicit `SpendPath`, the funded PSBT, assets and a
slice of `ProgramEvidenceAdapter` values. `ProgramEvidence` is the portable
implementation containing the exact requirement, codec and auxiliary bytes:

```rust,ignore
use emulator_connect::program::{prepare_spend, ProgramEvidence};

let evidence = [ProgramEvidence {
    requirement: selected_program,
    codec: "my-contract/evidence/v1".into(),
    witness: encoded_evidence,
}];
let prepared = prepare_spend(&artifact, selected_path, psbt, input_index, &assets, &evidence)?;
```

The codec must match the explicit capability. Preparation recomputes the plan;
it never trusts a serialized report. All prevouts must be supplied, unsupported
or contradictory branches fail, and every selected unsigned program needs its
own evidence. Unused adapters fail instead of preparing unrelated alternatives.
Missing native signatures and preimages remain visible in `prepared.plan`.

`prepared.program_requests` contains one request per selected unsigned program,
using the existing validated request protocol. Each request preserves native
PSBT assets and includes descriptor proofs. The application explicitly chooses
an oracle/evaluator, submits requests, verifies and merges responses, obtains
remaining native assets, and finalizes. Multiple responses start from a common
PSBT; replacing the whole PSBT with the last response would discard earlier
signatures.

## Weight and final checks

`satisfaction_weight_upper_bound` covers the selected input's serialized witness
and scriptSig, including their length prefixes. It excludes outpoint, sequence,
input-count and transaction overhead. The bound assumes the displayed stack
layout and current annex are retained; it is not a bound for an arbitrary future
witness. An absent signature reserves its maximum planned encoding size.
Descriptor bounds include witness/redeem scripts, nested scriptSig wrappers,
and legacy data-push and CompactSize prefixes. A legacy input reports zero
witness bytes; a transaction mixing legacy and Segwit inputs must additionally
count its serialized empty witness byte in whole-transaction accounting.

`observed_final_witness_bytes` measures supplied final witness data separately;
observing those bytes does not validate them. A `Planned` status likewise does
not establish signature validity, evaluator success, relay policy or maturity.
Use normal finalization, then `Template::check_funded_psbt` for retained funding
and final fee-rate constraints before extracting a transaction. The eltoo runner
demonstrates this ordering in `finalize_candidate`.
