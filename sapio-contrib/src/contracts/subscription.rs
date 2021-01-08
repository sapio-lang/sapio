use bitcoin::util::amount::CoinAmount;
use sapio::clause::Clause;
use sapio::contract::macros::*;
use sapio::contract::*;
use sapio::*;
use schemars::*;
use serde::*;
#[derive(JsonSchema, Serialize, Deserialize, Clone)]
struct Payment {
    amount: bitcoin::util::amount::CoinAmount,
    /// # Address
    /// The Address to send to
    address: bitcoin::Address,
}
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct TreePay {
    participants: Vec<Payment>,
    radix: usize,
}

use std::convert::TryInto;
impl TreePay {
    then! {expand |s, ctx| {
        let mut builder = ctx.template();
        if s.participants.len() > s.radix {

            for c in s.participants.chunks(s.participants.len()/s.radix) {
                let mut amt =  bitcoin::util::amount::Amount::from_sat(0);
                for Payment{amount, ..}  in c {
                    amt += amount.clone().try_into().map_err(|_| sapio::contract::CompilationError::TerminateCompilation)?;
                }
                builder = builder.add_output(amt.into(), &TreePay {participants: c.to_vec(), radix: s.radix}, None)?;
            }
        } else {
            for Payment{amount, address} in s.participants.iter() {
                builder = builder.add_output(*amount, &Compiled::from_address(address.clone(), None), None)?;
            }
        }
        Ok(Box::new(std::iter::once(builder.into())))
    }}
}

impl Contract for TreePay {
    declare! {then, Self::expand}
    declare! {non updatable}
}
