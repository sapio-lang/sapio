// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::Clause;
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;
use std::convert::TryInto;
use std::default::Default;
use std::fmt;
use std::marker::PhantomData;
use std::time::Duration;
/// Error in Creating a LockTime
#[derive(Debug)]
pub enum LockTimeError {
    /// Duration escapes bound of valid timestamps
    DurationTooLong(Duration),
    /// Time was too far in the past, would be interpreted as non-timestamp
    TimeTooFarInPast(Duration),
    /// height is too high (beyond 500_000_000), interpreted as timestamp
    HeightTooHigh(u32),
    /// sequence type is unknown
    UnknownSeqType(u32),
}

/// Type Tags used for creating lock time variants. The module lets us keep them
/// public while not polluting the name space.
pub mod type_tags {
    /// If the type is absolute or relative
    pub trait Absolutivity {
        /// true if type is absolute
        const IS_ABSOLUTE: bool;
    }
    ///if the type is height or time
    pub trait TimeType {
        /// true if type is height
        const IS_HEIGHT: bool;
    }
    use super::*;
    /// Type Tag for Realtive
    #[derive(Serialize, Deserialize, Copy, Clone, PartialOrd, Ord, Eq, PartialEq)]
    pub struct Rel;
    /// Type Tag for Absolute
    #[derive(Serialize, Deserialize, Copy, Clone, PartialOrd, Ord, Eq, PartialEq)]
    pub struct Abs;
    /// Type Tag for Height
    #[derive(Serialize, Deserialize, Copy, Clone, PartialOrd, Ord, Eq, PartialEq)]
    pub struct Height;
    /// Type Tag for Median Time Passed
    #[derive(Serialize, Deserialize, Copy, Clone, PartialOrd, Ord, Eq, PartialEq)]
    pub struct MTP;
}
use type_tags::*;

/// LockTime represents either a nLockTime or a Sequence field.
/// They are represented generically in the same type
#[derive(Serialize, Deserialize, Copy, Clone, PartialOrd, Ord, Eq, PartialEq)]
#[serde(transparent)]
pub struct LockTime<RelOrAbs: Absolutivity, HeightOrTime: TimeType>(
    u32,
    #[serde(skip)] PhantomData<(RelOrAbs, HeightOrTime)>,
);
#[derive(Serialize, Deserialize, Copy, Clone, PartialOrd, Ord, Eq, PartialEq)]
/// # Any Relative Time Lock
/// Represents a type which can be either type of relative lock
pub enum AnyRelTimeLock {
    /// # Relative Height
    /// in number of blocks
    RH(RelHeight),
    /// # Relative Time
    /// in chunks of 512 seconds
    RT(RelTime),
}

#[derive(Serialize, Deserialize, Copy, Clone, PartialOrd, Ord, Eq, PartialEq)]
/// # Any Absolute Time Lock
/// Represents a type which can be either type of absolute lock
pub enum AnyAbsTimeLock {
    /// # Absolute Height
    /// in exact block height
    AH(AbsHeight),
    /// # Absolute Time
    /// in unix time stamp since epoch
    AT(AbsTime),
}
#[derive(Serialize, Deserialize, Copy, Clone)]
/// # Any Time Lock (Relative, Absolute) x (Height, Time)
/// Represents a type which can be any type of lock
pub enum AnyTimeLock {
    /// # Relative
    R(AnyRelTimeLock),
    /// # Absolute
    A(AnyAbsTimeLock),
}

/// Helpful Aliases for specific concrete lock times
mod alias {
    use super::*;
    /// LockTime for Relative Height
    pub type RelHeight = LockTime<Rel, Height>;
    /// LockTime for Relative MTP
    pub type RelTime = LockTime<Rel, MTP>;
    /// LockTime for Absolute Height
    pub type AbsHeight = LockTime<Abs, Height>;
    /// LockTime for Absolute MTP
    pub type AbsTime = LockTime<Abs, MTP>;
    /// Maximum Date
    pub const BIG_PAST_DATE: AbsTime = LockTime(1_600_000_000u32, PhantomData);
    /// Minimum Date
    pub const START_OF_TIME: AbsTime = LockTime(500_000_000, PhantomData);
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
        /// get inner representation
        pub fn get(&self) -> u32 {
            self.0
        }
    }
    impl AnyRelTimeLock {
        /// get inner representation
        pub fn get(&self) -> u32 {
            match self {
                AnyRelTimeLock::RH(u) => u.get(),
                AnyRelTimeLock::RT(u) => u.get(),
            }
        }
    }

    impl AnyAbsTimeLock {
        /// get inner representation
        pub fn get(&self) -> u32 {
            match self {
                AnyAbsTimeLock::AH(u) => u.get(),
                AnyAbsTimeLock::AT(u) => u.get(),
            }
        }
    }

    impl AnyTimeLock {
        /// get inner representation
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
        fn try_from(t: u32) -> Result<Self, Self::Error> {
            if t < START_OF_TIME.get() {
                Err(LockTimeError::TimeTooFarInPast(Duration::from_secs(
                    t as u64,
                )))
            } else {
                Ok(Self(t, Default::default()))
            }
        }
    }
    impl TryFrom<u32> for AbsHeight {
        type Error = LockTimeError;
        fn try_from(u: u32) -> Result<Self, Self::Error> {
            if u < START_OF_TIME.get() {
                Ok(Self(u, Default::default()))
            } else {
                Err(LockTimeError::HeightTooHigh(u))
            }
        }
    }
    impl From<u16> for RelTime {
        fn from(u: u16) -> Self {
            // cast to wider type and then set bit 22 to specify relative time
            Self((u as u32) | (1 << 22), Default::default())
        }
    }
    impl From<u16> for RelHeight {
        fn from(u: u16) -> Self {
            // no bit setting required, direct cast to u32
            Self(u as u32, Default::default())
        }
    }

    impl TryFrom<Duration> for RelTime {
        type Error = LockTimeError;
        fn try_from(u: Duration) -> Result<Self, Self::Error> {
            u16::try_from(u.as_secs() / 512)
                .or(Err(LockTimeError::DurationTooLong(u)))
                .map(From::from)
        }
    }

    impl TryFrom<Duration> for AbsTime {
        type Error = LockTimeError;
        fn try_from(u: Duration) -> Result<Self, Self::Error> {
            u32::try_from(u.as_secs())
                .or(Err(LockTimeError::DurationTooLong(u)))?
                .try_into()
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
