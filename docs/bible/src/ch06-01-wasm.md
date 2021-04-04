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
    then!{
        fn demo(self, ctx) {
            let amt = ctx.funds()/2;
            ctx.template()
               .add_output(amt, &create_contract("users_cold_storage", /**/, amt), None)?
               .add_output(amt, &create_contract(&DEPENDS_ON_MODULE, /**/, amt), None)?
               .into()
        }
    }
}
```

### Future Work on Cross Module Calls

- **Type System:** Using JSONSchemas, plugins have a basic type system that
enables run-time checking for compatibility. Work could be done to establish
a trait based type system that can allow plugins to guarantee they implement
particular interfaces faithfully. For example, `users_cold_storage` key could
be wrapped in a type safe wrapper that knows how to respond to a
`ColdStorageArgs` struct.
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
