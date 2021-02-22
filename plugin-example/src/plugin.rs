use sapio::contract::*;
use sapio::*;
use sapio_wasm_plugin::client::*;
use sapio_wasm_plugin::*;
use schemars::*;
use serde::*;

#[derive(JsonSchema, Serialize, Deserialize, Clone)]
pub struct Payment {
    #[serde(with = "bitcoin::util::amount::serde::as_btc")]
    #[schemars(with = "f64")]
    pub amount: bitcoin::util::amount::Amount,
    /// # Address
    /// The Address to send to
    pub address: bitcoin::Address,
}
/// Documentation placed here will be visible to users!
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct TreePay {
    pub participants: Vec<Payment>,
    pub radix: usize,
}

impl TreePay {
    then! {expand |s, ctx| {
        let mut builder = ctx.template();
        if s.participants.len() > s.radix {

            for c in s.participants.chunks(s.participants.len()/s.radix) {
                let mut amt =  bitcoin::util::amount::Amount::from_sat(0);
                for Payment{amount, ..}  in c {
                    amt += *amount;
                }
                builder = builder.add_output(amt, &TreePay {participants: c.to_vec(), radix: s.radix}, None)?;
            }
        } else {
            for Payment{amount, address} in s.participants.iter() {
                builder = builder.add_output(*amount, &Compiled::from_address(address.clone(), None), None)?;
            }
        }
        builder.into()
    }}
}
impl Contract for TreePay {
    declare! {then, Self::expand}
    declare! {non updatable}
}
REGISTER![TreePay];
