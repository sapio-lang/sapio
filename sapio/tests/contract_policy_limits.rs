use bitcoin::blockdata::opcodes::all::OP_DROP;
use bitcoin::blockdata::script::Builder;
use bitcoin::{Amount, Network};
use sapio::contract::actions::Guard;
use sapio::contract::object::SupportedDescriptors;
use sapio::contract::{Compilable, CompilationError, Context, Contract};
use sapio::{declare, guard};
use sapio_base::covenant::LoweringPlan;
use sapio_base::policy::{PolicyError, ScriptFragment};
use std::cell::Cell;
use std::sync::Arc;

struct Alternatives<const COUNT: usize> {
    calls: Cell<usize>,
}

impl<const COUNT: usize> Alternatives<COUNT> {
    fn new() -> Self {
        Self {
            calls: Cell::new(0),
        }
    }

    #[guard(policy)]
    fn alternative(self, _ctx: Context) -> Result<ScriptFragment, PolicyError> {
        let call = self.calls.get() + 1;
        self.calls.set(call);
        // Each independent backend invocation produces a distinct complete
        // script. No signature generation is needed to exercise tree growth.
        ScriptFragment::new(
            Builder::new()
                .push_int(call as i64)
                .push_opcode(OP_DROP)
                .push_int(1)
                .into_script(),
        )
    }
}

impl<const COUNT: usize> Contract for Alternatives<COUNT> {
    const FINISH_FNS: &'static [fn() -> Option<Guard<Self>>] =
        &[Self::alternative as fn() -> Option<Guard<Self>>; COUNT];
    declare! {non updatable}
}

fn context() -> Context {
    Context::new(
        Network::Regtest,
        Amount::ZERO,
        LoweringPlan::Native,
        "limits".try_into().unwrap(),
        Arc::new(Default::default()),
        None,
    )
}

#[test]
fn the_contract_accepts_1024_independently_compiled_alternatives() {
    let contract = Alternatives::<1024>::new();
    let compiled = contract.compile(context()).unwrap();
    let Some(SupportedDescriptors::Taproot(raw)) = &compiled.descriptor else {
        panic!("raw backend branches must retain their spending data");
    };
    assert_eq!(raw.leaves().len(), 1024);
    assert_eq!(contract.calls.get(), 1024);
    compiled.validate().unwrap();
}

#[test]
fn the_contract_stops_evaluating_guards_when_the_branch_budget_is_exhausted() {
    let contract = Alternatives::<2048>::new();
    assert!(matches!(
        contract.compile(context()),
        Err(CompilationError::PolicyLimit {
            resource: "contract branches",
            limit: 1024,
        })
    ));
    // Reject the first excess emission instead of compiling every remaining
    // declared guard and checking the aggregate only at the end.
    assert_eq!(contract.calls.get(), 1025);
}
