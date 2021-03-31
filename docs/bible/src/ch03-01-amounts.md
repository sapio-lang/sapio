# Sats and Coins

There are several different ways of expressing amounts in Sapio.

That there isn't a single canonical way to represent amounts is unfortunate,
and hopefully these types can be fully unified in the future. But it's a
problem for good reason.

## A brief rant


Suppose I tell you to send 10 to Alice. Is that 10 sats? or 10 bitcoin? You
might think that 10.0 would be unambiguous, but it turns out the lightning
network is building sub-satoshi support.

The *only* way to make context-free unambiguous amounts is to have them
explicityly tagged, e.g., {denom: "sats", amount: 10}.

This would be great, but there are already myriads of services out there
where the only way to know what unit you have is to RTFM.

Generally, we know that floating point representations are evil for financial
transactions, but because we want to be compatible with JSON/Javascript, we
don't quite have a choice. Fortunately, 21e6 Bitcoin with 8 places fit
exactly into floats without loss. However, bets are off when doing arithmetic
with such values.

A last wrinkle: Bitcoin's amount type is a signed integer. Rust-bitcoin uses an Unsigned integer. So in theory there are unrepresentable amounts we're happy to work with. Great.

## It's up to every programmer

Therefore, to get amounts right is a task that is up to the programmer
largely to get this right. There are a few different amount types to be aware
of.

1. u64 represents sats. may be too big!
1. i64 represents sats. may be too small!
1. `bitcoin::Amount` represents u64, no standard serialization.
1. `bitcoin::SignedAmount` represents i64, no standard serialization.
1. `bitcoin::CoinAmount` standard tagged serialization, either u64 or f64.

These different types have uses in different circumstances.

Because `bitcoin::Amount` does not have a standard serializer, in order to
use it in e.g. a `Vec`, you have to wrap the type with a a serializer. `From` impls can make life a little eaiser to work with these.

```rust
use bitcoin::util::amount::Amount;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A wrapper around `bitcoin::Amount` to force it to serialize with f64.
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Ord, PartialOrd, PartialEq, Eq,
)]
#[serde(transparent)]
struct AmountF64(
    #[schemars(with = "f64")]
    #[serde(with = "bitcoin::util::amount::serde::as_btc")]
    Amount,
);

impl From<Amount> for AmountF64 {
    fn from(a: Amount) -> AmountF64 {
        AmountF64(a)
    }
}
impl From<AmountF64> for Amount {
    fn from(a: AmountF64) -> Amount {
        a.0
    }
}
```

`CoinAmount` does not have this problem, but it can't be used in all
contexts, e.g. extenral APIs that aren't tagged.


## Don't Panic (or do)

A final annoyance is that `bitcoin::Amount` has arithmetic that may panic
(unless you use the `checked_` variants). So one must be careful to ensure
that any set of values passed in are safe to add.

Sapio currently does not do a fantastic job of this, but that can be improved
in the future.
