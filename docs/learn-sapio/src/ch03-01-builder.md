# Template Builder

The Template builder is one of the most important parts of a Sapio contract.
It is how you define and build a transaction step.

It's also an area of active work to improve the UX of, to enable building new
kinds of smart contract more easily, supporting more advanced constructs.

The below code demonstrates how to use the template builder. See the
[docs](https://docs.rs/sapio/0.1.2/sapio/template/builder/struct.Builder.html)
for more detail!

```rust
struct X;
impl X {
    then! {
        fn example(self, ctx) {
            /// create a new template with the current context
            /// and set lock time to height 100
            let mut tmpl = ctx.template().set_lock_time(AbsHeight::from(10).into())?;
            let h = vec![(String::from("Metadata"), String::from("IS_COOL"))].into_iter().collect();
            /// Add an output
            /// make sure to assign to update after initial assignment, otherwise tmpl is consumed completely...
            /// Note: What happens when X creates an X (infinite loop)
            tmpl = tmpl.add_output(bitcoin::Amount::from_sat(1000), &X, Some(h))?;
            /// mark some funds unavailable (e.g. fees)
            tmpl = tmpl.spend_amount(bitcoin::Amount::from_sat(0xFEE))?;
            /// note that tmpl has it's own clone of ctx, which we should be
            /// careful to use instead of the passed in ctx, which is immutable
            if tmpl.ctx().funds() < bitcoin::Amount::from_sat(100000) {
                return Err(CompilationError::TerminateCompilation);
            }
            /// certain metadata is inteded to be "non-proprietary" and has dedicated setters
            tmpl = tmpl.set_label("Example!".into());
            /// adds a new _input_ and sets it sequence to relheight 1 block.
            tmpl = tmpl.add_sequence().set_sequence(-1, RelHeight::from(1))?;
            /// add some additional funds (i.e. from the input we just added)
            tmpl = tmpl.add_amount(Bitcoin::from_sats(10000));
            /// Send the remaining funds to this output
            tmpl = tmpl.add_output(tmpl.ctx().funds(), &X, None)?;
            let feeling_lazy = true;
            if feeling_lazy {
                /// This finishes the builder and turns it into the correct result type
                tmpl.into()
            } else {
                /// equivalently, but more verbosely
                Ok(Box::new(std::iter::once(Template::from(tmpl))))
            }
        }
    }
}
impl Contract for X {
    /*...*/
}

```


The Sapio model currently expects that all contracts UTXO spends are located
in the first input. The CTV hash commits to this, so it cannot be modified at
this time (but future work might allow changing this).
