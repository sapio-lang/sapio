# Concrete & Generic Types


## Generics

Often time, it can be useful to make a generic contract, such as:

```rust
struct GenericA {
    send_to: Box<dyn Compilable>
}
```
or
```rust
struct GenericB<T:Compilable> {
    send_to: T
}
```


In `GenericA` we use a _trait object_ to allow us to let the `send_to` field
equal any `Compilable` type while having having the same type `GenericA`,
whereas `GenericB` takes a type parameter that makes the `GenericB` more
specifically typed.

To highlight the differences between the approaches, suppose I had a parent contract:

```rust
#[derive(Serialize, Deserialize, JsonSchema)]
struct ConcreteA;
#[derive(Serialize, Deserialize, JsonSchema)]
struct ConcreteB;
struct AliceAndBobFree {
    alice: GenericA;
    bob: GenericA;
}
/// inner types can differ
let example_free_ok = AliceAndBobFree { alice: GenericA{send_to: Box::new(ConcreteA)},
                                        bob: GenericA{send_to:Box::new(ConcreteB)}};

struct AliceAndBobRestricted<T> {
    alice: GenericB<T>;
    bob: GenericB<T>;
}

/// inner types cannot differ
let example_restricted_fails = AliceAndBobRestricted { alice: GenericB{send_to: ConcreteA},
                                                       bob: GenericB{send_to: ConcreteB}};
```

It might _seem_ like you always want to use the `GenericA` variant, but there are cases where you
might want to guarantee that Alice and Bob's supplied contracts are the same type.

## Concrete Wrappers

When you do have a generic type (either with trait objects or otherwise) it
can be difficult to use across an application boundary. To get around this,
one can create a wrapper type (or enum) that uses the [`TryFrom`
paradigm](./ch08-03-concrete.md) to provide paths for the type to be concrete. E.g.,

```rust
#[derive(Serialize, Deserialize, JsonSchema)]
enum Concrete {
    A(ConcreteA),
    B(ConcreteB),
}

impl TryFrom<Concrete> for GenericA {
    type Error = &'static str;

    fn try_from(concrete:Concrete) -> Result<Self, Self::Error> {
        match concrete {
            Concrete::A(a) => GenericA(Box::new(a)),
            Concrete::B(b) => GenericA(Box::new(b))
        }
    }
}
```

Thus a `Concrete` can be used in a Serialize/Deserialize/JsonSchema API bound
context, whereas a `GenericA` could not.


### TODO: Implement path for making this section easier!