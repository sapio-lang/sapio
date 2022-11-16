// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! general non-parameter compilation state required by all contracts
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
use std::sync::Arc;
/// Used to Build a Shared Path for all children of a given context.
#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, PartialOrd, Ord, Eq)]
#[serde(try_from = "Y")]
#[serde(into = "Y")]
#[serde(
    bound = "T: Clone, Y: Serialize + for<'d> Deserialize<'d> + std::fmt::Debug + Clone, Y: Serialize + From<Self>, Self: TryFrom<Y>, <Self as TryFrom<Y>>::Error : std::fmt::Display"
)]
pub struct ReversePath<T, Y = String> {
    past: Option<Arc<ReversePath<T, Y>>>,
    this: T,
    _pd: PhantomData<Y>,
}

/// RPI = ReversePathIterator
/// This simplifies iterating over a reversepath.
pub struct RPI<'a, T, Y> {
    inner: Option<&'a ReversePath<T, Y>>,
}

impl<'a, T, Y> Iterator for RPI<'a, T, Y> {
    // we will be counting with usize
    type Item = &'a T;

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
impl<T, Y> TryFrom<Vec<T>> for ReversePath<T, Y> {
    type Error = &'static str;
    fn try_from(v: Vec<T>) -> Result<Self, Self::Error> {
        match v
            .into_iter()
            .fold(None, |x, v| Some(Self::push(x, v)))
            .map(Arc::try_unwrap)
        {
            Some(Ok(r)) => Ok(r),
            _ => Err("Reverse Path must have at least one element."),
        }
    }
}
impl<T: Clone, Y> From<ReversePath<T, Y>> for Vec<T> {
    fn from(r: ReversePath<T, Y>) -> Self {
        let mut v: Vec<T> = r.iter().cloned().collect();
        v.reverse();
        v
    }
}
impl<T: Clone, Y> From<T> for ReversePath<T, Y> {
    fn from(this: T) -> Self {
        ReversePath {
            past: None,
            this,
            _pd: Default::default(),
        }
    }
}
/// Helper for making a ReversePath.
pub struct MkReversePath<T, Y>(Option<Arc<ReversePath<T, Y>>>);
impl<T, Y> MkReversePath<T, Y> {
    /// Pop open a ReversePath, assuming one exists.
    pub fn unwrap(self) -> Arc<ReversePath<T, Y>> {
        if let Some(x) = self.0 {
            x
        } else {
            panic!("Vector must have at least one root path")
        }
    }
}
impl<T, Y> From<Vec<T>> for MkReversePath<T, Y> {
    fn from(v: Vec<T>) -> Self {
        let mut rp: Option<Arc<ReversePath<T, Y>>> = None;
        for val in v {
            let new: Arc<ReversePath<T, Y>> = ReversePath::push(rp, val);
            rp = Some(new);
        }
        MkReversePath(rp)
    }
}
impl<T, Y> ReversePath<T, Y> {
    /// Add an element to a ReversePath
    pub fn push(v: Option<Arc<ReversePath<T, Y>>>, s: T) -> Arc<ReversePath<T, Y>> {
        Arc::new(Self::push_owned(v, s))
    }
    /// Add an element to a ReversePath and do not wrap in Arc
    pub fn push_owned(v: Option<Arc<ReversePath<T, Y>>>, s: T) -> ReversePath<T, Y> {
        ReversePath::<T, Y> {
            past: v,
            this: s,
            _pd: Default::default(),
        }
    }
    /// iterate over a reversepath
    pub fn iter(&self) -> RPI<'_, T, Y> {
        RPI { inner: Some(self) }
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
                ReversePath::<i64, Vec<i64>>::push(Some(ReversePath::push(None, 1i64)), 5i64,)
                    .as_ref()
                    .clone()
            ),
            vec![1i64, 5]
        );
    }
    #[test]
    fn test_reverse_path_from_vec() {
        assert_eq!(
            ReversePath::<i64, Vec<i64>>::push(Some(ReversePath::push(None, 1i64)), 5,)
                .as_ref()
                .clone(),
            vec![1i64, 5].try_into().unwrap()
        );
    }
    #[test]
    fn test_reverse_path_into_serde() -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            sapio_data_repr::to_string(
                ReversePath::<i64, Vec<i64>>::push(Some(ReversePath::push(None, 1i64)), 5,)
                    .as_ref()
            )?,
            "[1,5]"
        );
        Ok(())
    }
    #[test]
    fn test_reverse_path_from_serde() -> Result<(), Box<dyn std::error::Error>> {
        let v: ReversePath<i64, Vec<i64>> = sapio_data_repr::from_str("[1,5]")?;
        assert_eq!(
            ReversePath::push(Some(ReversePath::push(None, 1i64)), 5,).as_ref(),
            &v
        );
        Ok(())
    }

    #[test]
    fn test_eq() {
        assert_eq!(
            ReversePath::<i64, Vec<i64>>::push(Some(ReversePath::push(None, 1i64)), 5,),
            ReversePath::push(Some(ReversePath::push(None, 1i64)), 5,)
        );
        let a = (0..100)
            .fold(None, |x, y| Some(ReversePath::<i64, Vec<i64>>::push(x, y)))
            .unwrap();
        assert_eq!(a, a.clone());
        let b = (0..100)
            .fold(None, |x, y| Some(ReversePath::push(x, y)))
            .unwrap();
        assert_eq!(a, b);
    }
    #[test]
    fn test_neq() {
        assert_ne!(
            ReversePath::<i64, Vec<i64>>::push(Some(ReversePath::push(None, 1i64)), 5,),
            ReversePath::push(Some(ReversePath::push(None, 0i64)), 5,)
        );
        let a = (0..100)
            .fold(None, |x, y| Some(ReversePath::<i64, Vec<i64>>::push(x, y)))
            .unwrap();
        let b = (0..101)
            .fold(None, |x, y| Some(ReversePath::<i64, Vec<i64>>::push(x, y)))
            .unwrap();
        assert_ne!(a, b);
    }
}
