# Time Locks

Sapio provides typed absolute and relative timelocks in
`sapio_base::timelocks`. Heights and times have separate constructors so a
value cannot silently change its interpretation.

```rust
use sapio_base::timelocks::*;
use sapio_base::Clause;
use std::convert::{TryFrom, TryInto};
use std::time::Duration;

# fn main() -> Result<(), LockTimeError> {
let height = AbsHeight::try_from(800_000u32)?;
let timestamp = AbsTime::try_from(1_000_000_000u32)?;
let same_timestamp = AbsTime::try_from(Duration::from_secs(1_000_000_000))?;

// Relative time is encoded in intervals of 512 seconds.
let intervals = RelTime::from(10u16);
let duration = RelTime::try_from(Duration::from_secs(10 * 512))?;
let blocks = RelHeight::from(20u16);

// Converting a transaction timelock into a policy is fallible.
let older: Clause = blocks.try_into()?;
let after: Clause = height.try_into()?;

let relative: AnyRelTimeLock = blocks.into();
let any: AnyTimeLock = relative.into();
let also_older = Clause::try_from(any)?;
# Ok(())
# }
```

`RelTime::try_from(Duration)` rounds up to whole 512-second intervals, so a
fractional interval cannot make the lock mature early. `AbsTime` rounds a
fractional timestamp up to a whole second. Both reject durations outside their
encoded range. The types' JSON values are their encoded consensus fields;
relative time includes the time-type flag, not just a count of seconds.

## Transaction fields and policy guards

Transaction fields and Miniscript predicates have different valid domains.
Sapio's absolute transaction locks cover heights `0..500_000_000` and timestamps
`500_000_000..=u32::MAX`. Relative heights and time-interval counts fit in a
`u16`. These types can be passed to the template builder without converting
them into policy clauses.

Miniscript's typed absolute guards accept encoded values `1..=0x7fff_ffff`.
Its relative guards require an enabled BIP68 encoding and a nonzero encoded
operand. For example, a zero-block sequence is a valid transaction field, but
cannot become an `Older` policy; a timestamp above `0x7fff_ffff` also cannot
become an `After` policy:

```rust
use sapio_base::timelocks::{AbsTime, RelHeight};
use sapio_base::Clause;

assert!(Clause::try_from(RelHeight::from(0u16)).is_err());
let timestamp = AbsTime::try_from(u32::MAX).unwrap();
assert!(Clause::try_from(timestamp).is_err());
```

Use `Clause::try_from(lock)` or `lock.try_into()` and propagate
`LockTimeError::InvalidPolicyLockTime` when building a guard. The `Any*TimeLock`
wrappers use the same fallible conversion. If constructing Miniscript clauses
directly, use its typed lock constructors; Sapio also rejects the exposed
`RelLockTime::ZERO` constant during policy validation.
