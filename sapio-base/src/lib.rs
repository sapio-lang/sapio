// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! base sapio library functionality and definitions, not particular to sapio compiler
#![deny(missing_docs)]
/// Extra functionality for working with Bitcoin types
pub mod util;
use std::{
    borrow::BorrowMut,
    cell::{Cell, RefCell},
    ops::DerefMut,
    rc::Rc,
};

use bitcoin::{blockdata::opcodes::all::OP_VERIFY, hashes::hex::FromHex, Script, XOnlyPublicKey};
use consts::TRUE_PATTERN;
pub use miniscript;
use miniscript::Tap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
pub use util::CTVHash;
pub mod plugin_args;
pub mod simp;

/// Helpers for making correct time locks
pub mod timelocks;
/// Trait & Structs for accessing Chain Data
pub mod txindex;

pub mod effects;
pub use effects::reverse_path;
pub mod serialization_helpers;

/// Any logical script clause
#[derive(Eq, Ord, Clone, PartialEq, PartialOrd, Debug, JsonSchema, Serialize, Deserialize)]
pub enum Clause {
    /// Script fragment (must end with 1 or 0 on stack)
    Script(bitcoin::Script),
    /// Logical Conjunction
    And(Box<RefCell<Clause>>, Box<RefCell<Clause>>),
    /// Logical Or
    Or(Box<RefCell<Clause>>, Box<RefCell<Clause>>, u64),
}

/// constants useful for various purposes
pub mod consts {
    // TODO: Fix the const fn version
    use bitcoin::blockdata::opcodes::all;
    ///pub const FALSE_PATTERN: &[u8] = &[all::OP_PUSHBYTES_0.into_u8()];
    pub const FALSE_PATTERN: &[u8] = &[0];
    ///pub const TRUE_PATTERN: &[u8] = &[all::OP_PUSHNUM_1.into_u8()];
    pub const TRUE_PATTERN: &[u8] = &[81];
}
impl Clause {
    /// Returns true if this [`Clause`] is an OP_TRUE
    pub fn is_trivial(&self) -> bool {
        match self {
            Clause::Script(s) => matches!(s.as_bytes(), consts::TRUE_PATTERN),
            _ => false,
        }
    }
    /// creates a trivial fragment
    pub fn trivial() -> Clause {
        Clause::Script(Script::from(TRUE_PATTERN.to_vec()))
    }
    ///
    pub fn wrap(self) -> Box<RefCell<Self>> {
        Box::new(RefCell::new(self))
    }
}
fn join_script_frags<'a, I: Iterator<Item = &'a Script>>(i: I) -> Script {
    let mut acc = vec![];
    for script in i {
        acc.extend_from_slice(script.as_bytes());
        acc.push(OP_VERIFY.into_u8());
    }
    // drop last verify
    acc.pop();
    Script::from_byte_iter(acc.iter().cloned().map(Ok)).unwrap()
}

fn assign_numbers_inner(p: &mut Clause, n: &mut u64) {
    match p {
        Clause::Script(v) => (),
        Clause::And(a, b) => {
            assign_numbers_inner(a.get_mut(), n);
            assign_numbers_inner(b.get_mut(), n);
        }
        Clause::Or(a, b, c) => {
            *c = *n;
            *n += 1;
        }
    }
}
fn assign_numbers(mut clause: Clause) -> (Clause, Vec<bool>) {
    let mut n = 0;
    assign_numbers_inner(&mut clause, &mut n);
    (clause, vec![false; n as usize])
}

fn compile_opt(clause: &mut Clause, opt: &[bool]) -> Script {
    match clause {
        Clause::Script(s) => s.clone(),
        Clause::And(a, b) => {
            join_script_frags([compile_opt(a.get_mut(), opt), compile_opt(b.get_mut(), opt)].iter())
        }
        Clause::Or(a, b, idx) => {
            if opt[*idx as usize] {
                compile_opt(a.get_mut(), opt)
            } else {
                compile_opt(b.get_mut(), opt)
            }
        }
    }
}

fn byte_to_bits(u: u8) -> [bool; 8] {
    [
        u & 1 > 0,
        u & 2 > 0,
        u & 4 > 0,
        u & 8 > 0,
        u & 16 > 0,
        u & 32 > 0,
        u & 64 > 0,
        u & 128 > 0,
    ]
}

/// Fail on complex scripts
#[derive(Debug)]
pub struct ScriptComplexityTooManyOrs;
impl Clause {
    /// generates all the leafs by letting each or be satisfied bit-by-bit
    pub fn generate_leafs(p: Self) -> Result<Vec<Script>, ScriptComplexityTooManyOrs> {
        let (mut clause, opt) = assign_numbers(p);
        if opt.len() > 16 {
            return Err(ScriptComplexityTooManyOrs);
        }
        let lim: u16 = 2u16.pow(opt.len() as u32);
        let mut out = vec![];
        for x in 0..lim {
            // double check the bit order TODO:
            let bytes = x.to_be_bytes();
            let v = [byte_to_bits(bytes[1]), byte_to_bits(bytes[0])].concat();
            out.push(compile_opt(&mut clause, &v[0..opt.len()]));
        }
        Ok(out)
    }
}

/// Concrete Instantiation of Miniscript Policy. Because we need to be able to generate exact
/// transactions, we only work with `bitcoin::PublicKey` types.
pub type Policy = miniscript::policy::concrete::Policy<XOnlyPublicKey>;

/// Concrete Instantiation of Miniscript Policy. Because we need to be able to generate exact
/// transactions, we only work with `bitcoin::PublicKey` types.
pub type Pol = miniscript::policy::concrete::Policy<XOnlyPublicKey>;

/// Helper for things that can become clauses
pub trait IntoClause {
    /// convert to clause
    fn to_clause(&self) -> Result<Clause, ()>;
}
impl IntoClause for Pol {
    fn to_clause(&self) -> Result<Clause, ()> {
        Ok(Clause::Script(
            self.compile::<Tap>().map_err(|_| ())?.encode(),
        ))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
