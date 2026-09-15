# Sapio Studio

[Sapio Studio](https://github.com/sapio-lang/sapio-studio) is an in-development graphical user
interface for Sapio.

Studio loads WASM modules and their argument/result schemas, connects compatible
value ports in a visual patch, and compiles the selected terminal module. These
wires describe how to construct public contract terms. The resulting transaction
graph is a separate view of what the compiled contract can spend to.

The [build-a-vault tutorial](https://github.com/sapio-lang/sapio/tree/master/contrib/build-a-vault)
provides ten small blocks and five executable patches. Start with signers,
destinations and a block delay; combine them into recovery and release rules;
then compile a fixed vault, a delayed wallet, or a dynamic OP_VAULT emulator.
The tutorial covers changing wires, inspecting the compiled transaction graph,
saving patches, and preparing the public evidence needed for signing.

Intermediate module results should use small, nonrecursive public data types.
The vault example shares the same Rust DTO between each output and compatible
input so schema matching stays precise. Terminal modules return `Compiled`;
private signing material stays outside the patch.
