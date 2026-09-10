# Typed covenant fragments for WASM-v2

Add this dependency to a `no_std` evaluator:

```toml
[dependencies]
sapio-covenant-fragments = { path = "../fragments" }
```

The library consumes the authenticated view supplied by the v2 runtime and
uses its deterministic crypto imports. A custom evaluator can compose helpers
directly without an AST, a Script stack, or another oracle request:

```rust,ignore
use sapio_covenant_fragments::{check_sig_from_stack, Context, Failure};

fn authorize(view: &[u8], signature: &[u8], scratch: &mut [u8])
    -> Result<bool, Failure>
{
    let context = Context::parse_v2(view)?;
    let message = context.template_hash(scratch)?;
    check_sig_from_stack(message.as_bytes(), context.internal_key(), signature)
}
```

`template_hash` returns a typed `TemplateHash` and requires at most
`MAX_VIEW_BYTES` of caller-owned scratch. It implements the
[BIP446](https://github.com/bitcoin/bips/blob/master/bip-0446.md) tagged hash,
including the selected input index and annex. Other inputs may spend legacy
outputs; scriptSig bytes do not enter this hash.

`check_sig_from_stack` also accepts arbitrary byte messages. It does not
implicitly hash them. An empty signature returns `Ok(false)`; a valid 64-byte
BIP340 signature returns `Ok(true)`. Malformed or invalid nonempty signatures
return a terminal `Failure`, preserving the typed 32-byte-key CSFS semantics.
Propagate errors with `?`; converting them to false would change the behavior
of negation and alternative branches. Rust conditionals remain conditional
evaluation, not eager Script Boolean opcodes.

`context.internal_key()` is the physical Taproot internal key authenticated by
the runtime. `context.output_key()` is the selected previous output's P2TR key.
`KnownTweak::from_slice(proof)?.authenticate(&context)?` returns a supplied key P
only after checking `Q = lift_x(P) + t*G` against that output key Q. The 65-byte
proof is `P[32] || t[32 big-endian] || parity[1]`; parity must be zero or one.
Then use CSFS to require a signature under P. This proves an additive relation;
it does not assert a particular TapTweak or BIP32 derivation. Never substitute
a witness-supplied Q for the output anchor.

Parsing arbitrary bytes cannot authenticate their provenance. `Context` is
intended for the runtime-supplied v2 view: the complete v1 projection followed
by the verified internal key, a four-byte little-endian annex length, and the
raw annex. Nonempty annexes begin with `0x50`. Truncation and trailing bytes
are rejected. The runtime checks the signing context before invoking the
evaluator; the decoder enforces its byte format.

V2 guests export `sapio_alloc_v2` and `sapio_evaluate_v2`. Their argument pairs
remain program, parameters, view, and witness. The SDK uses
`sapio_crypto_v1.sha256` and the v2 `schnorr_verify` and `xonly_tweak_check`
imports. The included `templatehash` and `template-authorization` programs show
the complete invocation and bounded scratch setup.
