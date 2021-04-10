# Sapio for Fun (and Profit)

In this section, we're going to build a simple option contract. This sort of
contract could be used, for example, to make an on-chain asynchronous offer to
someone to enter a bet with you.

Then, you'll have some challenges to modify the contract to extend it's
functionality meaningfully.

The logic for the basic contract is as follows:


1. If \\(\tau_{now} > \tau_{timeout} \\):
    - send funds to return address
1. If `strike_price` btc are added:
    - send funds + `strike_price` to strike_into contract


```rust
/// The Data Fields required to create a on-chain bet
pub struct UnderFundedExpiringOption {
    /// How much money has to be paid to strike the contract
    strike_price: Amount,
    /// if the contract expires, where to return the money
    return_address: bitcoin::Address,
    /// if the contract strikes, where to send the money
    strike_into: Box<dyn Compilable>,
    /// the timeout (as an absolute time) when the contract should end.
    timeout: AnyAbsTimeLock,
}

impl UnderFundedExpiringOption
{
    then! {
        /// return the funds on expiry
        fn expires(self, ctx) {
            Ok(Box::new(std::iter::once(
                ctx.template()
                    // set the timeout for this path -- because it is using
                    // then! we do not require a guard.
                    .set_lock_time(self.timeout)?
                    .add_output(
                        // ctx.funds() knows how much money has been sent to this contract
                        ctx.funds(),
                        // this bootstraps an address into a contract object
                        &Compiled::from_address(self.return_address.clone(), None),
                        None,
                    )?
                    .into(),
            )))
        }
    }

    then! {
        /// continue the contract
        fn strikes(self, ctx) {
            let mut tmpl = ctx.template().add_amount(self.strike_price);
            tmpl.add_sequence()
                .add_output(
                    /// use the inner context of tmpl because it has added funds
                    (tmpl.ctx().funds() + self.strike_price).into(),
                    &self.strike_into
                    None,
                )?
                .into()
        }
    }
}

impl Contract for UnderFundedExpiringOption
{
    declare!(then, Self::expires, Self::strikes);
    declare!(non updatable);
}
```

# Challenges

There's no right answer to the following challenges, and the resulting
contract may not be too useful, but it should be a good exercise to learn
more about writing Sapio contracts.

1. Write a contract designed to be put into the `strike_into` field which
sends funds to one party or the other based on a third-party revealing a
hash preimage `A` or `B`.
1. Modify the contract so that there is a `expire_A` and a `expire_B` path
that go to different addresses, and `expire_A` requires a signature or hash
reveal to be taken.
1. Modify the contract so that if `expire_A` is taken, a small payout
`early_exit_fee: bitcoin::Address` is made to a `early_exit :
bitcoin::Address`.
1. Modify the contract so that `expire_A` is only present the fields required
by it are `Option::is_some` (hint: use `compile_if!`).
1. Add logic to deduct fees.
1. Add a `cooperative_close` `guard!` clause that allows both parties to exit gracefully