// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Utilities for working with SIMPs (Sapio Interactive Metadata Protocols)

use std::{
    collections::BTreeMap,
    marker::PhantomData,
    ops::{ShlAssign, Shr},
    sync::Arc,
};

use bitcoin::psbt::serialize::Serialize;
use sapio_data_repr::SapioModuleBoundaryRepr;
use serde::Deserialize;
use std::fmt::Debug;

/// Errors that may come up when working with SIMPs
#[derive(Debug)]
pub enum SIMPError {
    /// If this SIMP is already present.
    /// Implementors may wish to handle or ignore this error if it is not an
    /// issue, but usually it is a bug.
    /// todo: Mergeable SIMPs may merge one another
    AlreadyDefined(SapioModuleBoundaryRepr),
    /// If the error was because a SIMP could not be serialized.
    ///
    /// If this error ever happens, your SIMP is poorly designed most likely!
    SerializationError(sapio_data_repr::Error),
}
impl std::fmt::Display for SIMPError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for SIMPError {}
impl From<sapio_data_repr::Error> for SIMPError {
    fn from(v: sapio_data_repr::Error) -> Self {
        SIMPError::SerializationError(v)
    }
}

/// Trait for Sapio Interactive Metadata Protocol Implementors
pub trait SIMP {
    /// Get a protocol number, which should be one that is assigned through the
    /// SIMP repo. Proprietary SIMPs can safely use negative numbers.
    fn static_get_protocol_number() -> i64
    where
        Self: Sized;
    /// Get a protocol number, which should be one that is assigned through the
    /// SIMP repo. Proprietary SIMPs can safely use negative numbers.
    ///
    /// Should be implementd as a pass throught to
    /// [`Self::static_get_protocol_number`], but the  trait system can't
    /// express that
    fn get_protocol_number(&self) -> i64;
    /// Conver a SIMP to a JSON. Concretely typed so that SIMP can be a trait object.
    fn to_sapio_data_repr(&self) -> Result<SapioModuleBoundaryRepr, sapio_data_repr::Error>;
    /// Conver a SIMP from a JSON. Sized bound so that SIMP can be a trait object.
    fn from_sapio_data_repr(value: SapioModuleBoundaryRepr) -> Result<Self, sapio_data_repr::Error>
    where
        Self: Sized;
}

/// Tag for where a SIMP may be validly injected
pub trait LocationTag {}

macro_rules! gen_location {
    ($x:ident) => {
        /// Type Tag for a SIMP Location
        pub struct $x;
        impl LocationTag for $x {}
    };
}

gen_location!(ContinuationPointLT);
gen_location!(CompiledObjectLT);
gen_location!(TemplateLT);
gen_location!(TemplateOutputLT);
gen_location!(GuardLT);
gen_location!(TemplateInputLT);

/// a trait a SIMP can implement to indicate where it should be able to be
/// placed
pub trait SIMPAttachableAt<T: LocationTag>
where
    Self: SIMP,
{
}

/// Trait Type Wrapper for Indexing with a SIMP.
///
/// Given a BTreeMap of SIMPs b, you can index it with the following syntax:
///
/// ```
/// use sapio_base::simp::SIMP;
/// use serde_json::Value;
/// use sapio_base::simp::by_simp;
/// use sapio_base::simp::simp_value;
/// use std::collections::BTreeMap;
/// struct MySimp(Value);
/// impl SIMP for MySimp {
/// fn static_get_protocol_number() -> i64
/// where
///     Self: Sized,
/// {
///     1234
/// }
/// fn get_protocol_number(&self) -> i64 {
///     Self::static_get_protocol_number()
/// }
/// fn to_json(&self) -> Result<Value, serde_json::Error> {
///     Ok(self.0.clone())
/// }
/// fn from_json(value: Value) -> Result<Self, serde_json::Error>
/// where
///     Self: Sized,
/// {
///     Ok(Self(value))
/// }
/// }
/// let mut b : BTreeMap<i64, Box<dyn SIMP>> = BTreeMap::new();
/// b <<= Box::new(MySimp("Howdy".into()));
/// if let Some(simp) = &b >> by_simp::<MySimp>() {
/// } else {
///     panic!();
/// }
/// let mut c : BTreeMap<i64, Value> = BTreeMap::new();
/// c <<= simp_value(MySimp("Howdy 2".into()));
/// if let Some(simp) = &c >> by_simp::<MySimp>() {
/// } else {
///     panic!();
/// }
/// ```
pub struct BySIMP<T>(pub PhantomData<T>);
/// Helper for indexing BySimp
pub fn by_simp<T>() -> BySIMP<T> {
    BySIMP(Default::default())
}

/// Wrapper to write to simp
pub struct SimpValue<T>(pub T);
/// Wrapper helper to write to simp
pub fn simp_value<T: SIMP>(v: T) -> SimpValue<T> {
    SimpValue(v)
}

impl<T: SIMP> ShlAssign<SimpValue<T>> for BTreeMap<i64, SapioModuleBoundaryRepr> {
    fn shl_assign(&mut self, rhs: SimpValue<T>) {
        if let Ok(js) = rhs.0.to_sapio_data_repr() {
            self.insert(T::static_get_protocol_number(), js);
        }
    }
}

impl ShlAssign<Box<dyn SIMP>> for BTreeMap<i64, Box<dyn SIMP>> {
    fn shl_assign(&mut self, rhs: Box<dyn SIMP>) {
        self.insert(rhs.get_protocol_number(), rhs);
    }
}

impl<'a, T: SIMP, V> Shr<BySIMP<T>> for &'a BTreeMap<i64, V> {
    type Output = Option<&'a V>;
    fn shr(self, _rhs: BySIMP<T>) -> Self::Output {
        self.get(&T::static_get_protocol_number())
    }
}

impl<'a, T: SIMP, V> Shr<BySIMP<T>> for &'a mut BTreeMap<i64, V> {
    type Output = Option<&'a mut V>;
    fn shr(self, _rhs: BySIMP<T>) -> Self::Output {
        self.get_mut(&T::static_get_protocol_number())
    }
}
