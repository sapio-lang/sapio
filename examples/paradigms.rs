use contract::*;
use sapio::*;

use libminiscript::policy::concrete::Policy;
use std::marker::PhantomData;
fn main() {
    println!("Hello, world!");
    let x: Channel<Start> = Channel {
        pd: PhantomData,
        state: Arc::new(Mutex::new(0)),
    };
    let y: Channel<Stop> = Channel {
        pd: PhantomData,
        state: Arc::new(Mutex::new(0)),
    };
    Compilable::compile(&x);
    Compilable::compile(&y);
}
use crate::clause::{Clause, SATISIFIABLE, UNSATISIFIABLE};

struct Start();
struct Stop();
trait State {}
impl State for Start {}
impl State for Stop {}
trait IsChannel<'a>
where
    Self: Sized + 'a,
{
    type MyState;
    const finishes: Option<ThenFunc<'a, Self>>;
    const finishes_or: Option<FinishOrFunc<'a, Self, FinishOrArgs>>;
    const finishes_or_2: Option<FinishOrFunc<'a, Self, FinishOrArgs>>;
}
use std::sync::{Arc, Mutex};
struct Channel<T: State> {
    pd: PhantomData<T>,
    state: Arc<Mutex<i64>>,
}

impl<'a> IsChannel<'a> for Channel<Start> {
    type MyState = Start;
    const finishes: Option<ThenFunc<'a, Self>> = None;
    const finishes_or: Option<FinishOrFunc<'a, Self, FinishOrArgs>> = None;
    const finishes_or_2: Option<FinishOrFunc<'a, Self, FinishOrArgs>> = None;
}
#[derive(Debug)]
enum FinishOrArgs {
    I(i64),
    S(String),
}
impl<'a> IsChannel<'a> for Channel<Stop> {
    type MyState = Stop;
    const finishes: Option<ThenFunc<'a, Self>> = Some(ThenFunc(
        &[Self::waits, Self::waits2, Self::waits3],
        |s: &Self| {
            println!("finishes for STOP");
            Box::new(std::iter::empty())
        },
    ));

    const finishes_or: Option<FinishOrFunc<'a, Self, FinishOrArgs>> = Some(
        FinishOrFuncNew(
            &[Self::waits, Self::waits2, Self::waits3],
            |s: &Self, o: Option<&FinishOrArgs>| {
                println!("finished_or for STOP");
                Box::new(std::iter::empty())
            },
        )
        .build(),
    );

    const finishes_or_2: Option<FinishOrFunc<'a, Self, FinishOrArgs>> = Some(
        FinishOrFuncNew(
            &[Self::waits, Self::waits2, Self::waits3],
            |s: &Self, o: Option<&FinishOrArgs>| {
                println!("finished_or_2 for STOP {:?}", o);
                Box::new(std::iter::empty())
            },
        )
        .build(),
    );
}
impl<'a, T: State + 'a> Channel<T> {
    const waits: Guard<Self> = Guard(
        |s: &Self| {
            let mut x = s.state.lock().unwrap();
            *x += 1;
            println!("Calling waits 1");
            UNSATISIFIABLE.clone()
        },
        false,
    );
    const waits2: Guard<Self> = Guard(
        |s: &Self| {
            println!("Calling waits 2");

            UNSATISIFIABLE.clone()
        },
        true,
    );
    const waits3: Guard<Self> = Guard(
        |s: &Self| {
            println!("Calling waits 3");
            UNSATISIFIABLE.clone()
        },
        true,
    );
    const continues: ThenFunc<'a, Self> = ThenFunc(&[Self::waits, Self::waits2], |s: &Self| {
        println!("continues for all");
        Box::new(std::iter::once(txn::Template::new().into()))
    });
}

impl<'a, T: State + 'a> Contract<'a> for Channel<T>
where
    Channel<T>: IsChannel<'a>,
{
    type StatefulArguments = FinishOrArgs;
    const THEN_FNS: &'a [Option<ThenFunc<'a, Self>>] =
        &[Some(Channel::continues), Channel::finishes];
    const FINISH_OR_FUNCS: &'a [Option<FinishOrFunc<'a, Self, Self::StatefulArguments>>] =
        &[Channel::<T>::finishes_or, Self::finishes_or_2];
}
