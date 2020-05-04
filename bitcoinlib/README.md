# Sapio

Welcome!

Sapio is a framework for creating composable multi-transaction Bitcoin Smart Contracts.

### Why is Sapio Different?
Sapio helps you build payment protocol specifiers that oblivious third parties
can participate in being none the wiser.

For example, with Sapio you can generate an address that represents a lightning
channel between you and friend and give that address to a third party service
like an exchange and have them create the channel without requiring any
signature interaction from you or your friend, zero trusted parties, and an
inability to differentiate your address from any other.

That's the tip of the iceberg of what Sapio lets you accomplish.


#### Say more...
Before Sapio, most Bitcoin smart contracts primarily focused on who can redeem
coins when and what unlocking conditions were required (see Ivy,
Policy/Miniscript, etc). A few languages, such as BitML, placed emphasis on
multi-transaction and multi-party use cases.

Sapio in particular focuses on transactions using BIP-119
OP_CHECKTEMPLATEVERIFY. OP_CHECKTEMPLATEVERIFY enables Bitcoin Script to support
complex multi-step smart contracts without a trusted setup. 

Sapio is a tool for defining such smart contracts in an easy way and exporting
easy to integrate APIs for managing open contracts. With Sapio you can turn what
previously would require months or years of careful tinkering with Bitcoin
internals into a 20 minute project and get a fully functional Bitcoin
application.

Sapio has intelligent built in features which help developers design safe smart
contracts and limit risk of losing funds.

### Show Me The Money! Sapio Crash Course:
Let's look at some example Sapio contracts (see
https://github.com/JeremyRubin/sapio/tree/master/sapio/examples for more
examples).


A Basic Pay to Public Key contract can be generated as follows:

```python
class PayToPublicKey(Contract):
    class Fields:
        key: PubKey
    @unlock(lambda self: SignatureCheckClause(self.key))
    def _(self): pass
```

Now let's look at an Escrow Contract. Here either Alice and Escrow, Bob and
Escrow, or Alice and Bob can spend the funds. Note that we use logic notation
where (|) is OR and (&) is and. These can also be written as `OrClause(a,b)` and
`AndClause(a,b)`.

```python
class BasicEscrow(Contract):
    class Fields:
        alice: PubKey
        bob: PubKey
        escrow: PubKey
    @unlock(lambda self: SignatureCheckClause(self.escrow) &\
        (SignatureCheckClause(self.alice) | SignatureCheckClause(self.bob)) | \
        (SignatureCheckClause(self.alice) & SignatureCheckClause(self.bob))
    )
    def redeem(self): pass
```

We can also write this a bit more clearly as:

```python
class BasicEscrow(Contract):
    class Fields:
        alice: PubKey
        bob: PubKey
        escrow: PubKey
    @unlock(lambda self: SignatureCheckClause(self.escrow) &\
        (SignatureCheckClause(self.alice) | SignatureCheckClause(self.bob))
    )
    def use_escrow(self): pass

    @unlock(lambda self: SignatureCheckClause(self.alice) & SignatureCheckClause(self.bob))
    def cooperate(self): pass
```

Until this point, we haven't made use of any of the CheckTemplateVerify
functionality of Sapio. These could all be done in Bitcoin today.

But Sapio lets us go further. What if we wanted to protect from Alice and the
escrow or Bob and the escrow from cheating?


```python
class TrustlessEscrow(Contract):
    class Fields:
        alice: PubKey
        bob: PubKey
        alice_escrow: Tuple[Amount, Contract]        
        bob_escrow: Tuple[Amount, Contract]        
    @path
    def use_escrow(self) -> TransactionTemplate:
        tx = TransactionTemplate()                    
        tx.add_output(*self.alice_escrow.assigned_value)
        tx.add_output(*self.bob_escrow.assigned_value)
        tx.set_sequence(Days(10).time)        
        return tx    

    @unlock(lambda self: SignatureCheckClause(self.alice) & SignatureCheckClause(self.bob))
    def cooperate(self): pass
```


Now with `TrustlessEscrow`, we've done a few things differently. A `@path`
designator tells the contract compiler to add a branch which *must* create the
returned transaction if that branch is taken.  We've also passed in a
sub-contract for both Alice and Bob to allow us to specify at a higher layer
what kind of pay out they receive. Lastly, we used a call to `set_sequence` to
specify that we should have to wait 10 days before using the escrow (we could
pass this as a parameter if we wanted though).

Thus we could construct an instance of this contract as follows:

```python
key_alice = #...
key_bob = #...
t = TrustlessEscrow(alice=key_alice,
                    bob=key_bob,
                    alice_escrow=(Bitcoin(1), PayToPublicKey(key=key_alice)),
                    bob_escrow=(Sats(10000), PayToPublicKey(key=key_bob)))
```

The power of Sapio becomes aparent when you look at the composability of the
framework. We can also put an escrow inside an escrow:


```python

key_alice = #...
key_bob = #...
t1 = TrustlessEscrow(alice=key_alice,
                    bob=key_bob,
                    alice_escrow=(Bitcoin(1), PayToPublicKey(key=key_alice)),
                    bob_escrow=(Sats(10000), PayToPublicKey(key=key_bob)))
t2 = TrustlessEscrow(alice=key_alice,
                    bob=key_bob,
                    alice_escrow=(Bitcoin(1), PayToPublicKey(key=key_alice)),
                    bob_escrow=(Sats(10000)+Bitcoin(1), t1))

# t3 throws an error because we would lose value
t3 = TrustlessEscrow(alice=key_alice,
                    bob=key_bob,
                    alice_escrow=(Bitcoin(1), PayToPublicKey(key=key_alice)),
                    bob_escrow=(Sats(10000), t1))
```

Sapio will look to make sure that all paths of our contract are sufficiently
funded, only losing an amount for fees (user configurable).


If you wanted to fund this contract from an exchange, all you need to do is request a withdrawal of the form:

```python
>>> print(t2.amount_range[1]/100e6, t2.witness_manager.get_p2wsh_address())
2.0001 bcrt1qh4ddpny622fcjf5m02nmdare7wgsuys5sau3jh5tdhm2kzwg9rzqw2sk00
```

#### TODO: Managing Smart Contracts

The wallet manager can be configured to save and watch a contract you create.

```
with Wallet() as w:
    w.watch(t2)
```

The wallet will automatically watch for payments to
bcrt1qh4ddpny622fcjf5m02nmdare7wgsuys5sau3jh5tdhm2kzwg9rzqw2sk00 and bind an
instance of the contract to the output.




# Getting Started With Sapio

#### TODO
