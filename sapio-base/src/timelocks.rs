use super::Clause;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;
use std::convert::TryInto;
use std::default::Default;
use std::fmt;
use std::marker::PhantomData;
use std::time::Duration;
#[derive(Debug)]
pub enum LockTimeError {
    DurationTooLong(Duration),
    TimeTooFarInPast(Duration),
    HeightTooHigh(u32),
    UnknownSeqType(u32),
}

/// Type Tags used for creating lock time variants. The module lets us keep them
/// public while not polluting the name space.
pub mod type_tags {
    pub trait Absolutivity {
        const IS_ABSOLUTE: bool;
    }
    pub trait TimeType {
        const IS_HEIGHT: bool;
    }
    use super::*;
    #[derive(JsonSchema, Serialize, Deserialize, Copy, Clone)]
    pub struct Rel;
    #[derive(JsonSchema, Serialize, Deserialize, Copy, Clone)]
    pub struct Abs;
    #[derive(JsonSchema, Serialize, Deserialize, Copy, Clone)]
    pub struct Height;
    #[derive(JsonSchema, Serialize, Deserialize, Copy, Clone)]
    pub struct MTP;
}
use type_tags::*;

/// LockTime represents either a nLockTime or a Sequence field.
/// They are represented generically in the same type
#[derive(JsonSchema, Serialize, Deserialize, Copy, Clone)]
pub struct LockTime<RelOrAbs: Absolutivity, HeightOrTime: TimeType>(
    u32,
    PhantomData<(RelOrAbs, HeightOrTime)>,
);
/// Represents a type which can be either type of relative lock
#[derive(JsonSchema, Serialize, Deserialize, Copy, Clone)]
pub enum AnyRelTimeLock {
    RH(RelHeight),
    RT(RelTime),
}

/// Represents a type which can be either type of absolute lock
#[derive(JsonSchema, Serialize, Deserialize, Copy, Clone)]
pub enum AnyAbsTimeLock {
    AH(AbsHeight),
    AT(AbsTime),
}
/// Represents a type which can be any type of lock
#[derive(JsonSchema, Serialize, Deserialize, Copy, Clone)]
pub enum AnyTimeLock {
    R(AnyRelTimeLock),
    A(AnyAbsTimeLock),
}

/// Helpful Aliases for specific concrete lock times
mod alias {
    use super::*;
    pub type RelHeight = LockTime<Rel, Height>;
    pub type RelTime = LockTime<Rel, MTP>;
    pub type AbsHeight = LockTime<Abs, Height>;
    pub type AbsTime = LockTime<Abs, MTP>;
}
pub use alias::*;

mod trait_impls {
    use super::*;
    impl Absolutivity for Rel {
        const IS_ABSOLUTE: bool = false;
    }
    impl Absolutivity for Abs {
        const IS_ABSOLUTE: bool = true;
    }
    impl TimeType for Height {
        const IS_HEIGHT: bool = true;
    }
    impl TimeType for MTP {
        const IS_HEIGHT: bool = false;
    }

    impl<A, TT> LockTime<A, TT>
    where
        A: Absolutivity,
        TT: TimeType,
    {
        pub fn get(&self) -> u32 {
            self.0
        }
    }
    impl AnyRelTimeLock {
        pub fn get(&self) -> u32 {
            match self {
                AnyRelTimeLock::RH(u) => u.get(),
                AnyRelTimeLock::RT(u) => u.get(),
            }
        }
    }

    impl AnyAbsTimeLock {
        pub fn get(&self) -> u32 {
            match self {
                AnyAbsTimeLock::AH(u) => u.get(),
                AnyAbsTimeLock::AT(u) => u.get(),
            }
        }
    }

    impl AnyTimeLock {
        pub fn get(&self) -> u32 {
            match self {
                AnyTimeLock::A(u) => u.get(),
                AnyTimeLock::R(u) => u.get(),
            }
        }
    }

    impl<A, TT> From<LockTime<A, TT>> for Clause
    where
        A: Absolutivity,
        TT: TimeType,
    {
        fn from(lt: LockTime<A, TT>) -> Clause {
            match (A::IS_ABSOLUTE, TT::IS_HEIGHT) {
                (true, true) => Clause::After(lt.0),
                (true, false) => Clause::After(lt.0),
                (false, true) => Clause::Older(lt.0),
                (false, false) => Clause::Older(lt.0),
            }
        }
    }

    impl fmt::Display for LockTimeError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{:?}", self)
        }
    }
    impl std::error::Error for LockTimeError {}

    impl TryFrom<u32> for AbsTime {
        type Error = LockTimeError;
        fn try_from(u: u32) -> Result<Self, Self::Error> {
            Ok(Self(u, Default::default()))
        }
    }
    impl TryFrom<u32> for AbsHeight {
        type Error = LockTimeError;
        fn try_from(u: u32) -> Result<Self, Self::Error> {
            if u < 500_000_000 {
                Ok(Self(u, Default::default()))
            } else {
                Err(LockTimeError::HeightTooHigh(u))
            }
        }
    }
    impl From<u16> for RelTime {
        fn from(u: u16) -> Self {
            Self((u as u32) | (1 << 22), Default::default())
        }
    }
    impl From<u16> for RelHeight {
        fn from(u: u16) -> Self {
            Self((u as u32) | (1 << 22), Default::default())
        }
    }

    impl TryFrom<Duration> for RelTime {
        type Error = LockTimeError;
        fn try_from(u: Duration) -> Result<Self, Self::Error> {
            let windows = u.as_secs() / 512;
            if windows > u16::MAX as u64 {
                Err(LockTimeError::DurationTooLong(u))
            } else {
                Ok((windows as u16).into())
            }
        }
    }

    impl TryFrom<Duration> for AbsTime {
        type Error = LockTimeError;
        fn try_from(u: Duration) -> Result<Self, Self::Error> {
            let t = u.as_secs();
            if t < 500_000_000 {
                Err(LockTimeError::TimeTooFarInPast(u))
            } else if t > u32::MAX as u64 {
                Err(LockTimeError::DurationTooLong(u))
            } else {
                (t as u32).try_into()
            }
        }
    }

    impl From<AnyRelTimeLock> for Clause {
        fn from(lt: AnyRelTimeLock) -> Self {
            match lt {
                AnyRelTimeLock::RH(a) => a.into(),
                AnyRelTimeLock::RT(a) => a.into(),
            }
        }
    }
    impl From<AnyAbsTimeLock> for Clause {
        fn from(lt: AnyAbsTimeLock) -> Self {
            match lt {
                AnyAbsTimeLock::AH(a) => a.into(),
                AnyAbsTimeLock::AT(a) => a.into(),
            }
        }
    }
    impl From<AnyTimeLock> for Clause {
        fn from(lt: AnyTimeLock) -> Self {
            match lt {
                AnyTimeLock::A(a) => a.into(),
                AnyTimeLock::R(a) => a.into(),
            }
        }
    }

    impl From<RelTime> for AnyRelTimeLock {
        fn from(lt: RelTime) -> Self {
            AnyRelTimeLock::RT(lt)
        }
    }
    impl From<AbsHeight> for AnyAbsTimeLock {
        fn from(lt: AbsHeight) -> Self {
            AnyAbsTimeLock::AH(lt)
        }
    }
    impl From<AbsTime> for AnyAbsTimeLock {
        fn from(lt: AbsTime) -> Self {
            AnyAbsTimeLock::AT(lt)
        }
    }

    impl From<RelHeight> for AnyRelTimeLock {
        fn from(lt: RelHeight) -> Self {
            AnyRelTimeLock::RH(lt)
        }
    }

    impl From<AnyAbsTimeLock> for AnyTimeLock {
        fn from(lt: AnyAbsTimeLock) -> Self {
            AnyTimeLock::A(lt)
        }
    }
    impl From<AnyRelTimeLock> for AnyTimeLock {
        fn from(lt: AnyRelTimeLock) -> Self {
            AnyTimeLock::R(lt)
        }
    }
}
