# Mempool & Fees

The Mempool is a treacherous place. If you're not familiar, the Mempool is
Bitcoin's backlog of unconfirmed transactions. It is a bounded queue which makes a best
effort at storing transactions that pay higher fees and dropping transactions which
pay insufficient fees.

The Mempool is an issue for a Sapio user because Sapio contracts are
generally immutable, which implies that Sapio contracts have to estimate the
minimum feerates at the time of contract creation.

For example, suppose I make a contract that has a state transition paying a
200 sats per vbyte feerate. And then by the time that transaction reaches the
mempool, it has gone up to 201 sats per vbyte minimum. Now I cannot easily
broadcast my transaction, and it is unlikely to wind up in a block.

There are many other ways that transactions can end up stuck.

Fortunately, there are some solutions to these sorts of problems, but none of
them are exactly "easy". We'll divide them in three categories:

# Careful Contract Programming

Careful contract programming can ensure that:

1. All contract transitions pay a high enough minimum we expect to be able to get into the mempool in the future
1. There are ways to inject "gas inputs" into the contract, if needed
1. There are ways to spend "gas outputs" from the contract just for Child-Pays-For-Parent logic.
1. Relative timelocks are used to prevent pinning attacks

For a discussion of this topic with visuals, please see the Sapio Reckless VR
Talk section on fees:

TODO: Integrate this content into writing

- [notes](https://diyhpl.us/wiki/transcripts/vr-bitcoin/2020-07-11-jeremy-rubin-sapio-101/)
- [slides](https://docs.google.com/presentation/d/1X4AGNXJ5yCeHRrf5sa9DarWfDyEkm6fFUlrcIRQtUw4/edit#slide=id.g8bddfc449f_0_358)
- [video](https://youtu.be/4vDuttlImPc?t=1665)

<div style="padding-bottom: 56.25%; position: relative;">
 <iframe style="position:absolute; top:0; left:0; width:100%; height: 100%;" src="https://www.youtube.com/embed/4vDuttlImPc?start=1665"
                                  frameborder="0" allow="accelerometer; autoplay; encrypted-media;
                                               gyroscope; picture-in-picture"
                                                  allowfullscreen></iframe>
</div>

# P2P Network/Mempool Policy Changes

Package Relay is a proposed technique that is progressing for Bitcoin whereby
multiple transactions can be submitted in one bundle to show suitability for
the mempool. Therefore a contract leaf node might be able to demonstrate, by
spending the coin, that the contract interior nodes are worth mining.

However, this technique is limited insofar as contract interior nodes in
Sapio may commonly have relative time locks (or similar) which prevent the
mempool from considering dependents.

Package Relaying does, however, improve the function of intentional gas outputs.

# Consensus Changes

Consensus changes are very difficult to create, but it's possible that in the
future some set of consensus changes help decouple contract execution from fee paying.

For example, there is a
[proposal](https://lists.linuxfoundation.org/pipermail/bitcoin-dev/2020-September/018168.html)
to replace Replace-By-Fee and Child-Pays-For-Parent with a mechanism that
functions as a virtual CPFP link. However, such proposals can introduce
subtle changes to Bitcoin's behavior and must be vetted closely.