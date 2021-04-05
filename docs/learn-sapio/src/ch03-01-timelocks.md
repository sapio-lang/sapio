# Time Locks


Sapio provides some utilities for working with both relative and absolute timelocks. See [the sapio-base docs](https://docs.rs/sapio-base/0.1.0/sapio_base/timelocks/index.html) for more details.


The Time Lock Utilities have some nice interfaces for dealing with timelocks generically and converting them into Policy Clauses.

```rust

use sapio_base::timelocks::*;
use std::time::Duration;

AbsHeight::try_from(800_000u32);
AbsTime::try_from(1_000_000_000u32);
AbsTime::try_from(Duration::from_secs(1_000_000_000u64));
// chunks of 512 seconds
RelTime::from(10u16);
RelTime::try_from(Duration::from_secs(10*512));
RelHeight::from(20u16);


// Correctly compiles into Clause::Older
let c: Clause = RelHeight::from(20u16).into();

let a: AnyRelTimeLock = RelHeight::from(20u16).into();
let b: AnyTimeLock = RelHeight::from(20u16).into();

```

These are not required to be used, but care should be taken if not used to
ensure that correct values are passed to the miniscript compiler.