# Sats and Coins

Sapio uses integer satoshis for transaction values. At a JSON boundary, the
interface must also make the denomination clear: `10` could otherwise mean
10 satoshis or 10 bitcoin.

## Units and serialization

These types serve different purposes:

| Type | Representation and JSON behavior |
| --- | --- |
| `u64` | An unsigned integer; the interface must specify its unit. |
| `i64` | A signed integer; the interface must specify its unit. |
| `bitcoin::Amount` | Unsigned integer satoshis. With Bitcoin's `serde` feature, its default JSON representation is an integer number of satoshis. |
| `bitcoin::SignedAmount` | Signed integer satoshis. Use an explicit `bitcoin::amount::serde` adapter for JSON fields. |
| `sapio_base::amount::CoinAmount` | A tagged input: `{"Sats": 1000}` or `{"Btc": 0.00001}`. |

`CoinAmount` belongs to Sapio, not Bitcoin. Convert it to `Amount` before using
it in a transaction. The `Btc` variant uses `Amount::from_btc` to check the
conversion, including range and fractional-satoshi precision; `Sats` preserves
the supplied integer exactly.

```rust
use bitcoin::Amount;
use sapio_base::amount::CoinAmount;

let amount = Amount::try_from(CoinAmount::Sats(1000)).unwrap();
assert_eq!(amount.to_sat(), 1000);
assert_eq!(serde_json::to_string(&amount).unwrap(), "1000");
```

A type's integer range is not a contract budget or Bitcoin's monetary limit.
Validate amounts against the funds available and the rules of the interface.

Binary floating point cannot represent most decimal bitcoin fractions exactly.
Prefer integer satoshis for arithmetic and JSON interfaces you control. In
JavaScript, integers up to `2^53 - 1` are exact, which covers Bitcoin's maximum
supply expressed in satoshis, but does not cover every `u64` value. JSON itself
does not require consumers to use floating point.

## Choosing a different wire format

`Amount` already works in a `Vec<Amount>` without a custom serializer. An
explicit wrapper is useful when an external interface requires another format,
such as a floating-point number of bitcoin. Its schema must describe that
chosen format too:

```rust
use bitcoin::Amount;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Serialize this interface's amounts as a number of bitcoin.
#[derive(
    Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Ord, PartialOrd, PartialEq, Eq,
)]
#[serde(transparent)]
struct AmountF64(
    #[schemars(with = "f64")]
    #[serde(with = "bitcoin::amount::serde::as_btc")]
    Amount,
);

impl From<Amount> for AmountF64 {
    fn from(amount: Amount) -> Self {
        Self(amount)
    }
}

impl From<AmountF64> for Amount {
    fn from(amount: AmountF64) -> Self {
        amount.0
    }
}
```

For ordinary satoshi fields in a `JsonSchema` type, use
`#[schemars(with = "u64")]` on `Amount`; upstream Bitcoin does not implement
`schemars::JsonSchema`. `bitcoin::amount::serde::as_sat` can explicitly document
the satoshi serialization, while `as_btc` changes it to bitcoin. Apply the same
choice consistently to the serializer, schema and interface documentation.

## Checked arithmetic

`Amount` arithmetic operators can panic on overflow or underflow. Use
`checked_add`, `checked_sub` and the other checked operations when values come
from callers, and propagate an error when a calculation cannot be represented.
Keep calculations in integer satoshis even when the wire format uses bitcoin.
