# Continuation Effects

Suppose we had the following bit of code in a contract's implementation:

```rust
#[derive(Serialize, Deserialize)]
struct PayToKey(bitcoin::PublicKey);
/// Helper
fn default_coerce(
    k: <T as Contract>::StatefulArguments,
) -> Result<PayToKey, CompilationError> {
    Ok(k)
}
/// A Guarded CTV Function
#[continuation(
    /// required: guards for the miniscript clauses required
    guarded_by = "[Self::guard_1,... Self::guard_n]",
    web_api,
    /// helper for coercing args for json api, could be arbitrary
    coerce_args = "default_coerce"
)]
fn to_address(self, ctx:Context, o:PayToKey) {
    let amt = ctx.funds();
    ctx.template().add_output(amt, &o.0, None)?.into()
}
```
When the `to_address` function gets passed by the compiler, a unique pointer (an effect path) is
generated for it from the context object. This enabled sending it parameters
later in the future.


On creation of the context object a `effects: Arc<MapEffectDB>` parameter  is
available.  This `MapEffectDB` links the effect paths to a list of arguments
intended to be passed to this branch which can generate new contract transitions
intended to be signed off on by the guards to that path.

For example, consider a contract for a NFT (a provenance checkable certificate
of ownership).

```rust
#[derive(Serialize, Deserialize)]
struct NFT(bitcoin::PublicKey);

#[derive(Serialize, Deserialize)]
struct Sale(bitcoin::PublicKey, AmountF64);
/// Helper
fn default_coerce(
    k: <T as Contract>::StatefulArguments,
) -> Result<Sale, CompilationError> {
    Ok(k)
}
impl NFT {
    #[guard]
    fn signed(self, ctx:Context) {
        Clause::Key(self.0)
    }
    #[continuation(
        guarded_by = "[Self::signed]",
        web_api,
        /// helper for coercing args for json api, could be arbitrary
        coerce_args = "default_coerce"
    )]
    fn make_sale(self, ctx:Context, o:Sale) {
        let amt = ctx.funds();
        ctx.template()
           .add_amount(o.1)
           // Carry whatever funds in the UTXO to the buyer in
           // a new NFT
           .add_output(amt, &NFT(o.0), None)?
           // Pay the sale amount to the previous owner
           .add_output(amt, &self.0, None)?
           .into()
    }
}
impl Contract for NFT {
    declare!{updatable<Sale>, Self::make_sale}
}
```

The updates generated through `make_sale` generate the transactions for a series
of sales. For example, imagine I start with a `NFT(Bob)`.

I can recompile `NFT(Bob)` with the context (not exactly the pointer, but just for example)
`NFT(Bob)` with effects
```json
{"0": {"make_sale": [["Alice", 10]]}}
```

Supposing Alice pays into the transaction, a future transaction could be:

```json
{"0": {"make_sale": [["Alice", 10]]}, "1": {"make_sale": [["Carol", 11]]}}
```

Thus by starting with a known valid NFT (e.g., Bob being the original artist),
the effects can regenerate a series of state transitions for verification of
provenance.


Further, as effects at a given point are a set, there could be multiple in flight transitions. E.g.,

```json
{"0": {"make_sale": [["Alice", 10], ["Eve", 10]]}}
```

represents Alice and Bob both having the ability to purchase the NFT for 10 BTC.


# Challenges

1. By using decreasing time locks, implement a dutch auction pitting two
participants against each other.
