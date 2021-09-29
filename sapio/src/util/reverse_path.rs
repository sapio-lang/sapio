// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! general non-parameter compilation state required by all contracts
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
/// Used to Build a Shared Path for all children of a given context.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
#[serde(try_from = "Vec<T>")]
#[serde(into = "Vec<T>")]
#[serde(bound = "T: Serialize + for<'d> Deserialize<'d> + JsonSchema + std::fmt::Debug + Clone ")]
pub struct ReversePath<T>
where
    T: Serialize + for<'d> Deserialize<'d> + JsonSchema + std::fmt::Debug + Clone,
{
    past: Option<Arc<ReversePath<T>>>,
    this: Arc<T>,
}

use std::convert::TryFrom;
impl<T> TryFrom<Vec<T>> for ReversePath<T>
where
    T: Serialize + for<'d> Deserialize<'d> + JsonSchema + std::fmt::Debug + Clone,
{
    type Error = &'static str;
    fn try_from(v: Vec<T>) -> Result<ReversePath<T>, Self::Error> {
        let mut rp = None;
        for val in v {
            rp = Some(ReversePath::push(rp, Arc::new(val)));
        }
        if let Option::Some(v) = rp {
            // Arc unwrap never fail!
            Ok(Arc::try_unwrap(v).unwrap())
        } else {
            Err("Reverse Path must have at least one element.")
        }
    }
}
impl<T> From<ReversePath<T>> for Vec<T>
where
    T: Serialize + for<'d> Deserialize<'d> + JsonSchema + std::fmt::Debug + Clone,
{
    fn from(r: ReversePath<T>) -> Self {
        let mut result: Vec<T> = vec![(*r.this).clone()];
        let mut node = &r.past;
        while let Some(v) = node {
            result.push((*v.this).clone());
            node = &v.past;
        }
        result.reverse();
        result
    }
}
impl<T> From<ReversePath<T>> for Vec<Arc<T>>
where
    T: Serialize + for<'d> Deserialize<'d> + JsonSchema + std::fmt::Debug + Clone,
{
    fn from(r: ReversePath<T>) -> Self {
        let mut result = vec![r.this];
        let mut node = &r.past;
        while let Some(v) = node {
            result.push(v.this.clone());
            node = &v.past;
        }
        result.reverse();
        result
    }
}
/// Helper for making a ReversePath.
pub struct MkReversePath<
    T: Serialize + for<'d> Deserialize<'d> + JsonSchema + std::fmt::Debug + Clone,
>(Option<Arc<ReversePath<T>>>);
impl<T> MkReversePath<T>
where
    T: Serialize + for<'d> Deserialize<'d> + JsonSchema + std::fmt::Debug + Clone,
{
    /// Pop open a ReversePath, assuming one exists.
    pub fn unwrap(self) -> Arc<ReversePath<T>> {
        if let Some(x) = self.0 {
            x
        } else {
            panic!("Vector must have at least one root path")
        }
    }
}
impl<T> From<Vec<Arc<T>>> for MkReversePath<T>
where
    T: Serialize + for<'d> Deserialize<'d> + JsonSchema + std::fmt::Debug + Clone,
{
    fn from(v: Vec<Arc<T>>) -> Self {
        let mut rp: Option<Arc<ReversePath<T>>> = None;
        for val in v {
            let new: Arc<ReversePath<T>> = ReversePath::push(rp, val);
            rp = Some(new);
        }
        MkReversePath(rp)
    }
}
impl<T> ReversePath<T>
where
    T: Serialize + for<'d> Deserialize<'d> + JsonSchema + std::fmt::Debug + Clone,
{
    /// Add an element to a ReversePath
    pub fn push(v: Option<Arc<ReversePath<T>>>, s: Arc<T>) -> Arc<ReversePath<T>> {
        Arc::new(ReversePath { past: v, this: s })
    }
}
