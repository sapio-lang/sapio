use super::undo_send::UndoSendInternal;
use bitcoin::util::amount::CoinAmount;

use sapio::contract::*;
use sapio::*;
use schemars::*;
use serde::*;
use std::convert::{TryFrom, TryInto};

use sapio_base::timelocks::AnyRelTimeLock;
use std::rc::Rc;

pub struct CoinPool {
    clauses: Vec<Clause>,
    refunds: Vec<(Compilable, Amount)>
}

impl CoinPool {
    then!(split_pool |s, ctx| {
        if clauses.len() >= 2 {
            let l = s.clauses.len();
            let a = CoinPool {
                clauses: s.clauses[0..l/2].into()
                refunds: s.refunds[0..l/2].into()
            };

            let b = CoinPool {
                clauses: s.clauses[l/2..].into()
                refunds: s.refunds[l/2..].into()
            };

            ctx.template().add_output(
                a.refunds.iter().map(|x| x.1).sum(),
                a,
                None
            ).add_output(
                b.refunds.iter().map(|x| x.1).sum(),
                b,
                None
            ).into()
        } else {
            let mut builder = ctx.builder();
            for (cmp, amt) in s.refunds {
            builder = builder.add_output(amt, cmp, None);
            }
            builder.into()
        }
    });
    guard! {all_approve |s, ctx| {Clause::Threshold(s.clauses.len(), s.clauses.clone())}};
}
impl Contract for CoinPool {
    declare! {then, Self::split_pool}
    declare! {finish, Self::all_approve}
}