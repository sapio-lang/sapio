// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Strings we only want to keep around once.
use std::sync::Arc;
use std::collections::HashMap;
lazy_static::lazy_static! {
    pub static ref CLONED : Arc<String> = Arc::new("cloned".into());
    pub static ref THEN_FN : Arc<String> = Arc::new("then_fn".into());
    pub static ref FINISH_OR_FN : Arc<String> = Arc::new("finish_or_fn".into());
    pub static ref FINISH_FN: Arc<String> = Arc::new("finish_fn".into());
    pub static ref CONDITIONAL_COMPILE_IF : Arc<String> = Arc::new("conditional_compile_if".into());
    pub static ref GUARD_FN : Arc<String> = Arc::new("guard_fn".into());
    pub static ref NEXT_TXS : Arc<String> = Arc::new("next_txs".into());
    pub static ref SUGGESTED_TXS : Arc<String> = Arc::new("suggested_txs".into());
    static ref INTERNED : HashMap<String, Arc<String>> = {
        let mut m = HashMap::<String, Arc<String>>::new();
        for s in [
            CLONED.clone(),
            THEN_FN.clone(),
            FINISH_OR_FN.clone(),
            FINISH_FN.clone(),
            CONDITIONAL_COMPILE_IF.clone(),
            GUARD_FN.clone(),
            NEXT_TXS.clone(),
            SUGGESTED_TXS.clone()]{
            m.insert(s.to_string(), s);
        }
        for i in 0..100 {
            m.insert(format!("{}", i),Arc::new(format!("{}", i)));
        }
        m
    };

}
pub fn get_interned(s: &str) -> Option<&Arc<String>> {
    (*INTERNED).get(&*s)
}