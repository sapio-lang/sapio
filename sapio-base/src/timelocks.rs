// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::Clause;
use schemars::JsonSchema;
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
    /// A valid transaction field cannot be represented by a Miniscript guard.
    InvalidPolicyLockTime(u32),
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
    #[derive(JsonSchema, Serialize, Deserialize, Copy, Clone, PartialOrd, Ord, Eq, PartialEq)]
    pub struct Rel;
    /// Type Tag for Absolute
    #[derive(JsonSchema, Serialize, Deserialize, Copy, Clone, PartialOrd, Ord, Eq, PartialEq)]
    pub struct Abs;
    /// Type Tag for Height
    #[derive(JsonSchema, Serialize, Deserialize, Copy, Clone, PartialOrd, Ord, Eq, PartialEq)]
    pub struct Height;
    /// Type Tag for Median Time Passed
    #[derive(JsonSchema, Serialize, Deserialize, Copy, Clone, PartialOrd, Ord, Eq, PartialEq)]
    pub struct MTP;
}
use type_tags::*;

/// LockTime represents either a nLockTime or a Sequence field.
/// They are represented generically in the same type
#[derive(Serialize, Copy, Clone, PartialOrd, Ord, Eq, PartialEq)]
#[serde(transparent)]
pub struct LockTime<RelOrAbs: Absolutivity, HeightOrTime: TimeType>(
    u32,
    #[serde(skip)] PhantomData<(RelOrAbs, HeightOrTime)>,
);
// The wire value is the encoded consensus field, including the relative-time
// flag. Deserialization must enforce the same domain as the smart constructors.
fn encoded_bounds<A: Absolutivity, TT: TimeType>() -> (u32, u32) {
    match (A::IS_ABSOLUTE, TT::IS_HEIGHT) {
        (false, true) => (0, u16::MAX as u32),
        (false, false) => (1 << 22, (1 << 22) | u16::MAX as u32),
        (true, true) => (0, 499_999_999),
        (true, false) => (500_000_000, u32::MAX),
    }
}
impl<'de, A: Absolutivity, TT: TimeType> Deserialize<'de> for LockTime<A, TT> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = u32::deserialize(deserializer)?;
        let (min, max) = encoded_bounds::<A, TT>();
        if !(min..=max).contains(&value) {
            return Err(serde::de::Error::custom(
                "Invalid encoded timelock for its height/time kind",
            ));
        }
        Ok(Self(value, PhantomData))
    }
}
impl<A: Absolutivity, TT: TimeType> JsonSchema for LockTime<A, TT> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        match (A::IS_ABSOLUTE, TT::IS_HEIGHT) {
            (false, true) => "RelativeHeight",
            (false, false) => "RelativeTime",
            (true, true) => "AbsoluteHeight",
            (true, false) => "AbsoluteTime",
        }
        .into()
    }
    fn json_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let (min, max) = encoded_bounds::<A, TT>();
        schemars::json_schema!({
            "type": "integer",
            "minimum": min,
            "maximum": max
        })
    }
}
#[derive(JsonSchema, Serialize, Deserialize, Copy, Clone, PartialOrd, Ord, Eq, PartialEq)]
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

#[derive(JsonSchema, Serialize, Deserialize, Copy, Clone, PartialOrd, Ord, Eq, PartialEq)]
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
#[derive(JsonSchema, Serialize, Deserialize, Copy, Clone)]
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

    impl<A, TT> TryFrom<LockTime<A, TT>> for Clause
    where
        A: Absolutivity,
        TT: TimeType,
    {
        type Error = LockTimeError;

        fn try_from(lt: LockTime<A, TT>) -> Result<Self, Self::Error> {
            if A::IS_ABSOLUTE {
                miniscript::AbsLockTime::from_consensus(lt.0)
                    .map(Clause::After)
                    .map_err(|_| LockTimeError::InvalidPolicyLockTime(lt.0))
            } else {
                miniscript::RelLockTime::from_consensus(lt.0)
                    .map(Clause::Older)
                    .map_err(|_| LockTimeError::InvalidPolicyLockTime(lt.0))
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
        /// Round up to whole 512-second intervals so the lock cannot mature
        /// earlier than the requested duration, including fractional seconds.
        fn try_from(u: Duration) -> Result<Self, Self::Error> {
            u16::try_from(u.as_nanos().div_ceil(512_000_000_000))
                .or(Err(LockTimeError::DurationTooLong(u)))
                .map(From::from)
        }
    }

    impl TryFrom<Duration> for AbsTime {
        type Error = LockTimeError;
        /// Round up to a whole Unix second so fractional timestamps cannot
        /// mature early; timestamps outside the consensus field are rejected.
        fn try_from(u: Duration) -> Result<Self, Self::Error> {
            u32::try_from(u.as_nanos().div_ceil(1_000_000_000))
                .or(Err(LockTimeError::DurationTooLong(u)))?
                .try_into()
        }
    }

    impl TryFrom<AnyRelTimeLock> for Clause {
        type Error = LockTimeError;
        fn try_from(lt: AnyRelTimeLock) -> Result<Self, Self::Error> {
            match lt {
                AnyRelTimeLock::RH(a) => a.try_into(),
                AnyRelTimeLock::RT(a) => a.try_into(),
            }
        }
    }
    impl TryFrom<AnyAbsTimeLock> for Clause {
        type Error = LockTimeError;
        fn try_from(lt: AnyAbsTimeLock) -> Result<Self, Self::Error> {
            match lt {
                AnyAbsTimeLock::AH(a) => a.try_into(),
                AnyAbsTimeLock::AT(a) => a.try_into(),
            }
        }
    }
    impl TryFrom<AnyTimeLock> for Clause {
        type Error = LockTimeError;
        fn try_from(lt: AnyTimeLock) -> Result<Self, Self::Error> {
            match lt {
                AnyTimeLock::A(a) => a.try_into(),
                AnyTimeLock::R(a) => a.try_into(),
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

#[cfg(test)]
mod wire_tests {
    use super::*;
    fn bounds<T: JsonSchema + serde::de::DeserializeOwned + serde::Serialize>(min: u32, max: u32) {
        for raw in [min, max] {
            let lock: T = serde_json::from_value(raw.into()).unwrap();
            assert_eq!(serde_json::to_value(lock).unwrap(), raw);
        }
        for invalid in [u64::from(max) + 1, u64::MAX] {
            assert!(serde_json::from_value::<T>(invalid.into()).is_err());
        }
        if min > 0 {
            assert!(serde_json::from_value::<T>((min - 1).into()).is_err());
        }
        assert!(serde_json::from_str::<T>("-1").is_err());
        let schema = schemars::schema_for!(T);
        assert_eq!(schema.as_value()["minimum"], min);
        assert_eq!(schema.as_value()["maximum"], max);
    }
    #[test]
    fn all_four_encoded_domains_match_the_schema() {
        bounds::<RelHeight>(0, 65_535);
        bounds::<RelTime>(1 << 22, (1 << 22) | 65_535);
        bounds::<AbsHeight>(0, 499_999_999);
        bounds::<AbsTime>(500_000_000, u32::MAX);
    }
    #[test]
    fn transaction_fields_and_policy_guards_have_distinct_domains() {
        for value in [0, 1, 65_535] {
            let lock = RelHeight::from(value as u16);
            let policy = Clause::try_from(lock);
            assert_eq!(policy.is_ok(), value != 0);
            if let Ok(Clause::Older(relative)) = policy {
                assert_eq!(relative.to_consensus_u32(), value);
            }
        }
        for value in [500_000_000, i32::MAX as u32, u32::MAX] {
            let lock: AbsTime = serde_json::from_value(value.into()).unwrap();
            assert_eq!(lock.get(), value);
            let policy = Clause::try_from(lock);
            assert_eq!(policy.is_ok(), value <= i32::MAX as u32);
            if let Ok(Clause::After(absolute)) = policy {
                assert_eq!(absolute.to_consensus_u32(), value);
            }
        }
        assert!(Clause::try_from(AbsHeight::try_from(0).unwrap()).is_err());
        let time = RelTime::from(42);
        assert!(matches!(Clause::try_from(time), Ok(Clause::Older(lock))
            if lock.to_consensus_u32() == (1 << 22) | 42));
    }
    #[test]
    fn duration_conversion_never_rounds_a_lock_earlier() {
        for (duration, intervals) in [
            (Duration::ZERO, 0),
            (Duration::from_nanos(1), 1),
            (Duration::from_secs(1), 1),
            (Duration::from_secs(512), 1),
            (Duration::new(512, 1), 2),
            (Duration::from_secs(864_000), 1688),
            (Duration::from_secs(65_535 * 512), 65_535),
        ] {
            assert_eq!(
                RelTime::try_from(duration).unwrap().get(),
                (1 << 22) | intervals
            );
        }
        assert!(RelTime::try_from(Duration::new(65_535 * 512, 1)).is_err());
        assert!(RelTime::try_from(Duration::MAX).is_err());
        assert_eq!(
            AbsTime::try_from(Duration::new(600_000_000, 1))
                .unwrap()
                .get(),
            600_000_001
        );
        assert_eq!(
            AbsTime::try_from(Duration::from_secs(u32::MAX as u64))
                .unwrap()
                .get(),
            u32::MAX
        );
        assert!(AbsTime::try_from(Duration::new(u32::MAX as u64, 1)).is_err());
        assert!(AbsTime::try_from(Duration::MAX).is_err());
        assert!(AbsTime::try_from(Duration::from_secs(499_999_999)).is_err());
    }

    #[test]
    fn relative_flags_cannot_disable_or_change_lock_kind() {
        for value in [1 << 16, 1 << 22, 1 << 31, u32::MAX] {
            assert!(serde_json::from_value::<RelHeight>(value.into()).is_err());
        }
        for value in [0u32, 1 << 16, (1 << 22) | (1 << 16), (1 << 22) | (1 << 31)] {
            assert!(serde_json::from_value::<RelTime>(value.into()).is_err());
        }
        let time = RelTime::from(42);
        let json = serde_json::to_value(time).unwrap();
        assert_eq!(json, (42 | (1 << 22)));
        assert_eq!(
            serde_json::from_value::<RelTime>(json).unwrap().get(),
            time.get()
        );
    }
}
