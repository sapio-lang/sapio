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
    T: JsonSchema + std::fmt::Debug + Clone,
{
    past: Option<Arc<ReversePath<T>>>,
    this: Arc<T>,
}

impl<T> PartialEq for ReversePath<T>
where
    T: PartialEq,
    T: JsonSchema + std::fmt::Debug + Clone,
{
    fn eq(&self, other: &Self) -> bool {
        iter(self).eq(iter(other))
    }
}
impl<T> Eq for ReversePath<T>
where
    T: Eq,
    T: JsonSchema + std::fmt::Debug + Clone,
{
}

/// RPI = ReversePathIterator
/// This simplifies iterating over a reversepath.
pub struct RPI<'a, T>
where
    T: JsonSchema + std::fmt::Debug + Clone,
{
    inner: Option<&'a ReversePath<T>>,
}

fn iter<'a, T>(s: &'a ReversePath<T>) -> RPI<'a, T>
where
    T: JsonSchema + std::fmt::Debug + Clone,
{
    RPI {
        inner: Some(s),
    }
}

impl<'a, T> Iterator for RPI<'a, T>
where
    T: JsonSchema + std::fmt::Debug + Clone,
{
    // we will be counting with usize
    type Item = &'a Arc<T>;

    // next() is the only required method
    fn next(&mut self) -> Option<Self::Item> {
        let ret = self.inner.map(|x| &x.this);
        match self.inner.map(|x| x.past.as_ref()) {
            Some(Some(x)) => {
                self.inner = Some(x);
            }
            _ => {
                self.inner = None;
            }
        }
        ret
    }
}

use std::convert::TryFrom;
impl<T> TryFrom<Vec<T>> for ReversePath<T>
where
    T: Serialize + for<'d> Deserialize<'d> + JsonSchema + std::fmt::Debug + Clone,
{
    type Error = &'static str;
    fn try_from(v: Vec<T>) -> Result<ReversePath<T>, Self::Error> {
        v.into_iter()
            .map(Arc::new)
            .fold(None, |x, v| Some(ReversePath::push(x, v)))
            .map(Arc::try_unwrap)
            .map(Result::unwrap)
            .ok_or("Reverse Path must have at least one element.")
    }
}
impl<T> From<ReversePath<T>> for Vec<T>
where
    T: JsonSchema + std::fmt::Debug + Clone,
{
    fn from(r: ReversePath<T>) -> Self {
        let mut v: Vec<T> = iter(&r).map(|s: &Arc<T>| s.as_ref().clone()).collect();
        v.reverse();
        v
    }
}
impl<T> From<ReversePath<T>> for Vec<Arc<T>>
where
    T: JsonSchema + std::fmt::Debug + Clone,
{
    fn from(r: ReversePath<T>) -> Self {
        let mut v: Vec<Arc<T>> = iter(&r).map(|s: &Arc<T>| s.clone()).collect();
        v.reverse();
        v
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

#[cfg(test)]
mod test {
    use super::*;
    use std::convert::TryInto;
    #[test]
    fn test_reverse_path_into_vec() {
        assert_eq!(
            Vec::<i64>::from(
                ReversePath::push(Some(ReversePath::push(None, Arc::new(1i64))), Arc::new(5),)
                    .as_ref()
                    .clone()
            ),
            vec![1i64, 5]
        );
    }
    #[test]
    fn test_reverse_path_from_vec() {
        assert_eq!(
            ReversePath::push(Some(ReversePath::push(None, Arc::new(1i64))), Arc::new(5),)
                .as_ref()
                .clone(),
            vec![1i64, 5].try_into().unwrap()
        );
    }
    #[test]
    fn test_reverse_path_into_serde() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            serde_json::to_string(
                ReversePath::push(Some(ReversePath::push(None, Arc::new(1i64))), Arc::new(5),)
                    .as_ref()
            )?,
            "[1,5]"
        );
        Ok(())
    }
    #[test]
    fn test_reverse_path_from_serde() -> Result<(), Box<dyn std::error::Error>> {
        let v: ReversePath<i64> = serde_json::from_str("[1,5]")?;
        assert_eq!(
            ReversePath::push(Some(ReversePath::push(None, Arc::new(1i64))), Arc::new(5),).as_ref(),
            &v
        );
        Ok(())
    }

    #[test]
    fn test_eq() {
        assert_eq!(
            ReversePath::push(Some(ReversePath::push(None, Arc::new(1i64))), Arc::new(5),),
            ReversePath::push(Some(ReversePath::push(None, Arc::new(1i64))), Arc::new(5),)
        );
        let a = (0..100).map(Arc::new).fold(None, |x,y| Some(ReversePath::push(x, y))).unwrap();
        assert_eq!(a, a.clone());
        let b = (0..100).map(Arc::new).fold(None, |x,y| Some(ReversePath::push(x, y))).unwrap();
        assert_eq!(a, b);
    }
    #[test]
    fn test_neq() {
        assert_ne!(
            ReversePath::push(Some(ReversePath::push(None, Arc::new(1i64))), Arc::new(5),),
            ReversePath::push(Some(ReversePath::push(None, Arc::new(0i64))), Arc::new(5),)
        );
        let a = (0..100).map(Arc::new).fold(None, |x,y| Some(ReversePath::push(x, y))).unwrap();
        let b = (0..101).map(Arc::new).fold(None, |x,y| Some(ReversePath::push(x, y))).unwrap();
        assert_ne!(a, b);
    }
}
