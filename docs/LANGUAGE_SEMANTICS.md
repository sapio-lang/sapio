# Contract action semantics

Sapio's Rust frontend declares guarded transaction transitions and continuation
entry points. The compiler must preserve their authorization through Bitcoin
lowering, even when several transitions produce the same transaction.
Native Miniscript clauses and custom policy backends share those action rules;
see [policy backends](POLICY_BACKENDS.md) for the extension API, raw-script
obligations, compilation budgets and artifact schema changes.

## Declarations

`#[then]`, `#[continuation]`, `#[guard]` and `#[compile_if]` accept only their
documented options. Unknown names, duplicates, malformed expressions and
unsupported method signatures are compile errors at the declaration. In
particular, misspelling `guarded_by` or `compile_if` cannot silently remove a
restriction. Explicit return types, method visibility and attributes survive
expansion. The shorthand `self` receiver becomes the shared reference required
by the callback API.

The `decl_*` trait macros declare the same callback interfaces as their matching
attributes. An optional declaration returns `None` until implemented. Qualified
`sapio::declare!` calls work without importing the macro. Continuation schema
helpers preserve the method's case, and raw Rust identifiers use their ordinary
name in effect paths.

## Guards and transitions

For a CTV action with action guard `A`, a returned template with additional guard
`T` contributes the spending condition:

```text
A AND T AND covenant(template_hash)
```

Every action contributes its own condition before transaction storage is
deduplicated. If Alice can authorize a transaction directly, while Bob needs
Carol's approval for that same transaction, the result is:

```text
(Alice OR (Bob AND Carol)) AND covenant(template_hash)
```

Bob alone remains insufficient regardless of action order. Tests evaluate the
lowered descriptor for every signer combination, with native CTV and an emulated
covenant key, and with a matching or different template commitment.

All listed guards are conjunctive. Zero effective guards mean true, one means
that guard, and larger native lists use valid binary or threshold policy nodes.
Exact duplicate ordinary native predicates and trivial clauses do not add
requirements. Inscription clauses carry script data: conjunction preserves their
order and multiplicity, including when nested inside another predicate. Raw
policy fragments also preserve order and multiplicity; Sapio combines them with
verification instructions between predicates. A continuation
contributes its declared guards, while its returned transactions are suggestions;
**every** suggested template is checked for forbidden additional guards before
deduplication.

The compiled template's `additional_preconditions` includes the action guards.
For a shared transaction it records their complete alternative conditions in
canonical policy order. The committed descriptor or raw Taproot tree remains the
authority for spending.

## Source validity and possible transitions

Native policy nodes are validated before simplification or alternative
splitting. Invalid arities, thresholds, time constants and inscription fields
remain errors even when another predicate would make their branch unreachable.
Keys may occur in separate alternatives such as `(A AND B) OR (A AND C)`;
Miniscript checks each eventual native script. General `ScriptPolicy::And` and
`ScriptPolicy::Or` permit n-ary lists, while native `Clause::And` and `Clause::Or`
require binary nodes. A contract that produces no spending branches returns
`EmptyPolicy`.

For a committed transition, native constraints must at least be possible for
the template's fixed transaction fields. Contradictions involving the committed
input's version, sequence, locktime or CTV hash produce `ImpossibleTemplate`
with its hash and action path. Keys and preimages are treated as potentially
available. This check does not establish chain maturity, funding availability
or complete satisfiability, and it does not infer the meaning of raw scripts.

## Duplicate transaction payloads

A CTV hash does not commit to funding budgets, metadata or child continuation
paths. Two templates can share a stored transaction only when their complete
binding payloads agree: transaction, commitment index, funding/fee requirements,
input metadata, template metadata and output contract graphs. A disagreement
returns `ConflictingTemplate` with the hash, action/effect path and differing
field. Authorizations may differ and are combined as alternatives.

Authors intentionally sharing a transaction should reuse a common compiled
destination and metadata. Equal child addresses alone are insufficient: their
continuation APIs and source paths must also agree. The compiler never chooses
a child graph, label or fee requirement solely because it appeared first.

## Cached clauses and contextual metadata

Fresh guards receive `(self, Context)` at each attachment. Cached guards use
`#[guard(cached)] fn signed(self)`: their clause callback has no invocation
context and is evaluated once per compilation of that contract. At the Rust API
boundary this is `Guard::Cache(fn(&Self) -> Clause, ...)`.

Custom guards opt in with `#[guard(policy)]` and an explicit backend return
type. Adding `cached` likewise removes `Context`; the helper and its backend
translation run once per contract compilation. Fresh backend guards translate
at every attachment. Backend failures propagate as compilation errors.

Guard metadata callbacks always receive their actual attachment context, for
both cached and fresh clauses. Finish guards contribute metadata as well as
spending conditions. Equal serialized metadata values for a policy/protocol
are deduplicated; distinct values from different attachments are retained in
declaration order. Allocation addresses never control metadata ordering.
The artifact stores these as ordered policy records with per-protocol values,
so native and custom guard sources retain the same metadata semantics.

Caching does not make arbitrary Rust code pure. Reproducible compilation still
requires contract authors to derive behavior from explicit inputs rather than
mutable external state.

## Conditional compilation and effect paths

Conditions are evaluated in declaration order. Their path indexes preserve
declared slots, including absent factories. Derivation failures are returned;
the compiler does not search for another free path or skip an error.

| Decision | Action behavior |
| --- | --- |
| NoConstraint / Required | Execute; a CTV action must produce a template |
| Nullable | Execute; an empty CTV action contributes no spending branch |
| Skippable / Never | Skip the action body and its guards |
| Fail | Stop compilation with the supplied diagnostics |

Across a condition list, `Required` overrides `Skippable` and `Nullable`, while
`Never` conflicts with `Required`. Explicit failures retain declaration order;
the complete list also reports that contradiction once. Decision kinds form an
associative, commutative merge; ordered diagnostic lists do not.

Named action/effect fragments use ASCII letters, digits and underscores.
Reserved fragments and separators cannot be passed as user names. All effect
names for a continuation are checked before its default or JSON callback runs,
so the callback cannot observe a path that changes meaning after serialization.
Repeated action names receive deterministic suffixes in the current declaration
order; changing the set or order of action factories can change those paths.

Effects augment the default continuation invocation. They do not replace it.
These rules describe compiler behavior; use artifact validation and binding
checks before consuming publicly constructed or deserialized compiled objects.
