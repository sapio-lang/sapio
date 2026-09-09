# WASM

WASM is "WebAssembly", or a standard for producing bytecode objects that can
be run on any platform. As the name suggests, it was originally designed for
use in web browsers as a compiler target for any language to produce code to
run safely from untrusted sources.

So what's it doing in Sapio?

WASM is designed to be cross platform and deterministic, which makes it a
great target for smart contracts that we want to be able to be reproduced
locally. Sapio validates guest memory access and applies execution fuel,
memory/table caps and nested-call limits before running a module. These bounds
cover guest execution; native compilation and external services need their own
resource policy. Loading a module is not a guarantee that its contract is safe
or that its intended covenant is enforced on the selected chain.

Sapio Contract objects can be built into  WASM binaries very easily. The code required is basically:

```rust
/// MyContract must support Deserialize and JsonSchema
#[derive(Deserialize, JsonSchema)]
struct MyContract;
impl Contract for MyContract{\*...*\};
/// binds to the plugin interface -- only one REGISTER macro permitted per project
REGISTER![MyContract];
```

See [the example](https://github.com/sapio-lang/sapio/tree/master/plugin-example) for more details.

These compiled objects require a special environment to be interacted with.
That environment is provided by the [Sapio CLI](./ch07-00-cli.md) as a
standalone binary. It is also possible to use the interface provided by the
`sapio-wasm-plugin` crate to load a plugin from your rust codebase
programmatically. Lastly, one could create similar bindings for another
platform as long as a WASM interpreter is available.


## Cross Module Calls

The WASM Plugin Handle architecture permits one WASM plugin to call into
another. This is incredibly powerful. What this enables one to do is to
package Sapio contracts that are generic and can call one another either by
hash (with effective subresource integrity) or by a nickname (providing easy
user customizability).

For example, suppose I was writing a standard contract component `C` which I
publish. Then later, I develop a contract `B` which is designed to work with
`C`. Rather than having to depend on `C`'s source code (which I may not want
to do for various reasons), I could simply hard code `C`'s hash into `B` and
call `create_contract_by_key(key: &[u8; 32], args: Value, amt: Amount)` to
get the desired code. The plugin management system automatically searches for
a contract plugin with that hash, and tries to call it with the provided JSON
arguments. Using `create_contract(key:&str, args:Value: amt:Amount)`, a
nickname can be provided in which case the appropriate plugin is resolved by
the environment.


```rust
struct C;
const DEPENDS_ON_MODULE : [u8; 32] = [0;32];
impl Contract for C {
    #[then]
    fn demo(self, ctx: Context) {
        let amt = ctx.funds()/2;
        ctx.template()
            .add_output(amt, &create_contract("users_cold_storage", /**/, amt), None)?
            .add_output(amt, &create_contract(&DEPENDS_ON_MODULE, /**/, amt), None)?
            .into()
    }
}
```
### Typed Calls

`SapioHostAPI<T, R>` resolves a module locator to a key and provides typed calls.
Arguments `T` implement `Serialize`, `JsonSchema`, and `Clone`; results `R`
implement `Deserialize` and `JsonSchema`. Resolving the locator makes no claim
that every value of `T` is accepted by that module.

The native host validates each actual `CreateArgs<T>` input against the module's
advertised input schema before calling its create function. It validates each
successful result against the advertised output schema before returning it,
then the caller deserializes that result as `R`. Ordinary module errors remain
errors. These checks enforce JSON constraints for that call; contract behavior
and compatibility between whole interfaces require their own specifications.

Versioned enum variants identify shared calling conventions. For example, the
batching interface defines its arguments as follows:

```rust
/// A payment to a specific address
#[derive(JsonSchema, Serialize, Deserialize, Clone)]
pub struct Payment {
    /// The amount to send
    #[serde(with = "bitcoin::util::amount::serde::as_btc")]
    #[schemars(with = "f64")]
    pub amount: bitcoin::util::amount::Amount,
    /// # Address
    /// The Address to send to
    pub address: bitcoin::Address,
}
#[derive(Serialize, JsonSchema, Deserialize, Clone)]
pub struct BatchingTraitVersion0_1_1 {
    pub payments: Vec<Payment>,
    #[serde(with = "bitcoin::util::amount::serde::as_sat")]
    #[schemars(with = "u64")]
    pub feerate_per_byte: bitcoin::util::amount::Amount,
}
```

The shared interface wraps those arguments in a versioned variant:

```rust
#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub enum Versions {
    BatchingTraitVersion0_1_1(BatchingTraitVersion0_1_1),
}

pub type BatchingModule = ContractModule<Versions>;
```

`ContractModule<Versions>` is a `SapioHostAPI` whose result is a compiled contract.
The serialized arguments keep the version tag:

```json
{"BatchingTraitVersion0_1_1":{"payments":[],"feerate_per_byte":0}}
```

The `treepay` example accepts this variant alongside its direct `TreePay` and
`Advanced` variants. A receiver can therefore offer additional calling
conventions while accepting the shared batching input. Its schema is checked
against the actual call rather than compared for equality with the caller's
schema.

### Future Work on Cross Module Calls



- **Gitian Packaging:** Using a gitian signed packaging distribution system
would enable a user to set up a web-of-trust setting for their sapio compiler
and enable fetching of sub-resources by hash if they've been signed by the
appropriate parties.
- **NameSpace Registration:** A system to allow people to register names
unambiguously would aid in ensuring no conflicts. For now, we can handle
this using a centralized repo.
- **Remote CMC:** In some cases, we may want to make a call to a remote
server that will call a given module for us. This might be desirable if the
server holds sensitive material that we shouldn't have.
