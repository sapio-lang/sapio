use bitcoin::util::amount::CoinAmount;
use sapio::clause::Clause;
use sapio::contract::macros::*;
use sapio::contract::*;
use sapio::*;
use schemars::*;
use serde::*;

#[derive(JsonSchema, Serialize, Deserialize)]
pub struct ExampleA {
    alice: bitcoin::PublicKey,
    bob: bitcoin::PublicKey,
    amount: CoinAmount,
    resolution: Compiled,
}

pub struct Args;
impl<'a> ExampleA {
    guard!(timeout | s | { Clause::Older(100) });
    guard!(cached signed |s| {Clause::And(vec![Clause::Key(s.alice), Clause::Key(s.bob)])});
}

impl<'a> Contract<'a> for ExampleA {
    def! {finish, Self::signed, Self::timeout}
    def! {updatable<Args> }
}

use std::marker::PhantomData;
pub trait BState: JsonSchema {
    fn get_n(n: u8, max: u8) -> u8 {
        return max;
    }
}
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct Start;
impl BState for Start {}
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct Finish;
impl BState for Finish {
    fn get_n(n: u8, max: u8) -> u8 {
        return n;
    }
}

trait ExampleBThen<'a>
where
    Self: Sized + 'a,
{
    then! {begin_contest}
}

#[derive(JsonSchema, Serialize, Deserialize)]
pub struct ExampleB<T: BState> {
    participants: Vec<bitcoin::PublicKey>,
    threshold: u8,
    amount: CoinAmount,
    #[serde(skip)]
    pd: PhantomData<T>,
}

impl<'a, T: BState> ExampleB<T> {
    guard!(timeout | s | { Clause::Older(100) });
    guard!(cached all_signed |s| {Clause::Threshold(T::get_n(s.threshold, s.participants.len()as u8) as usize, s.participants.iter().map(|k| Clause::Key(*k)).collect())});
}

impl<'a> ExampleBThen<'a> for ExampleB<Finish> {}
impl<'a> ExampleBThen<'a> for ExampleB<Start> {
    then! {begin_contest |s| {
        let o = txn::Output::new(
            s.amount,
            ExampleB::<Finish> {
                participants: s.participants.clone(),
                threshold: s.threshold,
                amount: s.amount,
                pd: Default::default(),
            },
            None,
        )?;
        Ok(Box::new(std::iter::once(
            txn::TemplateBuilder::new().add_output(o).into(),
        )))
    }}
}

impl<'a, T: BState + 'a> Contract<'a> for ExampleB<T>
where
    ExampleB<T>: ExampleBThen<'a>,
{
    def! {then, Self::begin_contest}
    def! {finish, Self::all_signed}
    def! {updatable<Args> }
}

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
impl<'a> TreePay {
    then! {expand |s| {
        let mut builder = txn::TemplateBuilder::new();
        if s.participants.len() > s.radix {

            for c in s.participants.chunks(s.participants.len()/s.radix) {
                let mut amt =  bitcoin::util::amount::Amount::from_sat(0);
                for Payment{amount, ..}  in c {
                    amt += amount.clone().try_into().map_err(|_| sapio::contract::CompilationError::TerminateCompilation)?;
                }
                builder = builder.add_output(txn::Output::new(amt.into(), TreePay {participants: c.to_vec(), radix: s.radix}, None)?);
            }
        } else {
            for Payment{amount, address} in s.participants.iter() {
                builder = builder.add_output(txn::Output::new(*amount, Compiled::from_address(address.clone(), None), None)?);
            }
        }
        Ok(Box::new(std::iter::once(builder.into())))
    }}
}

impl<'a> Contract<'a> for TreePay {
    def! {then, Self::expand}
    def! {updatable<Args>}
}
