# WASM

WASM is "WebAssembly", or a standard for producing bytecode objects that can
be run on any platform. As the name suggests, it was originally designed for
use in web browsers as a compiler target for any language to produce code to
run safely from untrusted sources.

So what's it doing in Sapio?

WASM is designed to be cross platform and deterministic, which makes it a
great target for smart contracts that we want to be able to be reproduced
locally. It also makes it *relatively* safe to run smart contracts provided
by untrusted parties as the security of the WASM sandbox prevents bad code from
harming or infecting our system.

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
 Using JSONSchemas, plugins have a basic type system that enables run-time
 checking for compatibility. Plugins can guarantee they implement particular
 interfaces faithfully. These interfaces currently only support protecting the
 call, but make no assurances about the returned value or potential errors from
 the callee's implementation of the trait.

For example, suppose I want to be able to specify a provided module must
statisfy a calling convention for batching. I define the trait
`BatchingTraitVersion0_1_1` as follows:

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

I can then turn this into a SapioJSONTrait by implementing the trait and
providing an "example" function.
```rust
impl SapioJSONTrait for BatchingTraitVersion0_1_1 {
    /// required to implement
    fn get_example_for_api_checking() -> Value {
        #[derive(Serialize)]
        enum Versions {
            BatchingTraitVersion0_1_1(BatchingTraitVersion0_1_1),
        }
        serde_json::to_value(Versions::BatchingTraitVersion0_1_1(
            BatchingTraitVersion0_1_1 {
                payments: vec![],
                feerate_per_byte: bitcoin::util::amount::Amount::from_sat(0),
            },
        ))
        .unwrap()
    }

    /// optionally, this method may be overridden directly for more advanced type checking.
    fn check_trait_implemented(api: &dyn SapioAPIHandle) -> bool {
        Self::check_trait_implemented_inner(api).is_ok()
    }
}
```
If a contract module can receive the example, then it is considered to have
implemented the API. We can implement the receivers for a module as follows:

```rust
struct MockContract;
/// # Different Calling Conventions to create a Treepay
#[derive(Serialize, Deserialize, JsonSchema)]
enum Versions {
    /// # Base
    Base(MockContract),
    /// # Batching Trait API
    BatchingTraitVersion0_1_1(BatchingTraitVersion0_1_1),
}
impl From<BatchingTraitVersion0_1_1> for MockContract {
    fn from(args: BatchingTraitVersion0_1_1) -> Self {
        MockContract
    }
}
impl From<Versions> for TreePay {
    fn from(v: Versions) -> TreePay {
        match v {
            Versions::Base(v) => v,
            Versions::BatchingTraitVersion0_1_1(v) => v.into(),
        }
    }
}
REGISTER![[MockContract, Versions], "logo.png"];
```

Now `MockContract` can be called via the `BatchingTraitVersion0_1_1` trait
interface.

Another module in the future need only have a field
`SapioHostAPI<BatchingTraitVersion0_1_1>`. This type verifies at deserialize
time that the provided name or hash key implements the required interface(s).

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
- **Concrete CMC:** currently, CMC's only return the `Compiled` type. Perhaps
future `CMC` support can return arbitrary types, allowing other types of functionality
to be packaged.
