use crate::clause::Clause;
use crate::contract::macros::*;
use crate::contract::*;
use crate::*;
use bitcoin::util::amount::CoinAmount;
use schemars::*;
use serde::*;
#[derive(JsonSchema, Serialize, Deserialize, Clone)]
pub struct Payment {
    pub amount: bitcoin::util::amount::CoinAmount,
    /// # Address
    /// The Address to send to
    pub address: bitcoin::Address,
}
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct TreePay {
    pub participants: Vec<Payment>,
    pub radix: usize,
}

use std::convert::TryInto;
impl TreePay {
    then! {expand |s| {
        let mut builder = template::Builder::new();
        if s.participants.len() > s.radix {

            for c in s.participants.chunks(s.participants.len()/s.radix) {
                let mut amt =  bitcoin::util::amount::Amount::from_sat(0);
                for Payment{amount, ..}  in c {
                    amt += amount.clone().try_into().map_err(|_| crate::contract::CompilationError::TerminateCompilation)?;
                }
                builder = builder.add_output(template::Output::new(amt.into(), &TreePay {participants: c.to_vec(), radix: s.radix}, None)?);
            }
        } else {
            for Payment{amount, address} in s.participants.iter() {
                builder = builder.add_output(template::Output::new(*amount, &Compiled::from_address(address.clone(), None), None)?);
            }
        }
        builder.into()
    }}
}

impl Contract for TreePay {
    declare! {then, Self::expand}
    declare! {non updatable}
}
