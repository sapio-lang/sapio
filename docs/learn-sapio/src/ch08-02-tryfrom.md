# TryFrom Constructors

Often times we want to assure that various properties must be true about the
arguments passed to a contract instance.

By using TryFrom and being careful with the visibility of inner fields it is
possible to guarantee that the only way to get an X is by going through type
Y.

This can be bound using the `serde(try_from)` attribute, which makes it so
that any deserialization of `X` first passes through `Y`. This is
particularly useful when `X` contains types (such as function pointers or
caches) that cannot be deserialized, but we want to provide a way for a third
party to pass JSON args to construct an `X`.

```rust
use std::convert::TryFrom;
use std::convert::TryInto;
use serde::*;
/// inner argument not pub, X cannot be constructed without going through Y
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(try_from="Y")]
pub struct X(u32);

#[derive(Serialize, Deserialize, JsonSchema)]
pub struct Y(pub u32);
impl TryFrom<Y> for X {
    type Error = &'static str;
    fn try_from(y: Y) -> Result<Self, Self::Error> {
        if y.0 < 10 {
            Err("Too Small I Guess?")
        } else {
            Ok(X(y.0))
        }
    }
}

let x: X = Y(10).try_into().unwrap();

```
