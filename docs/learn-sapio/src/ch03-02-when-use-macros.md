# When to use macros?

Generally, you want to use `finish!`, `then!`, etc to generate your methods.
However, if you prefer to create them manually, it's entirely possible to do
so without much effort. A tool like `cargo expand` may be useful as you can
just copy the macro output and customize from there.

One reason you might choose to manually define them is if you want to have
custom static logic (that is, known just from the type and not a value-filled
instance) to decide if a method should be `Some` or `None`. If it does not
need to be static logic, a `compile_if` can be used.
