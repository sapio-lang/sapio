# Sapio CTV Emulators


Sapio CTV Emulators defines implementations of the emulator trait that can
be used by sapio compiler library users. This includes wrapper types that
compose instances of an emulator into a federated multisig.

This crate also defines logic for servers that want to offer emulator services.

See [Sapio CLI](../cli/README.md) for how to run a server.

## Connection lifecycle

The default client deadline is 30 seconds for each peer's signing exchange. It
includes waiting for that connection's previous request, connecting, sending the
request, reading the complete response and validating that only signatures were
added. Only a successfully validated exchange returns its socket to the cache.
Timeout, invalid response or cancellation during an exchange discards the socket;
the next call connects again. A request that times out while queued leaves the
previous request's socket alone.

`HDOracleEmulatorConnection::with_request_timeout` changes this allowance. The
CLI's `covenant.request_timeout_secs` defaults to 30 and also supplies a
separate deadline for awaiting resolution of the complete peer configuration.
Blocking system resolver work can continue after that deadline and delay runtime
shutdown. Library callers
using the asynchronous connection constructor directly must bound its DNS
resolution themselves. Federation signing visits peers sequentially, with a
separate request deadline for each peer. A federation with N peers can therefore
spend N times the configured allowance on peer exchanges.

The server admits at most 64 live connections by default. At capacity it stops
accepting sockets; additional clients wait in the operating system's backlog.
Time spent in that backlog is outside the server request allowance. Each admitted
connection has 30 seconds per request, including idle time, the complete frame
header/body and the response write. Partial bytes do not restart the deadline.
A successfully completed response starts a fresh allowance for the next request.
JSON frames remain limited to one million bytes.

Create a server with `HDOracleEmulator::new(root)`, and override its policy with
`.with_limits(request_timeout, max_connections)`. Both limits must be positive
and the timeout must fit the runtime's clock. The CLI exposes these as
`--request-timeout-secs` and `--max-connections`. The former debug/`--sync` mode
has been removed: a malformed request or disconnected peer closes its connection
without terminating the listener. Completed tasks are reaped; cancelling the
server aborts its active connection tasks. Listener failures and unexpected task
failures are returned to the caller.

These are I/O and connection-admission limits. Async timers cannot interrupt
synchronous parsing, cryptography or response validation, and cancellation of
running native work takes effect when it yields. They do not establish a hard
CPU or process-memory budget for a public deployment.


## How it works

*See the source code for more detailed documentation.*

CheckTemplateVerify essentially functions as a self-signed transaction. I.e.,
imagine you could create a public key that could only ever sign a transaction
which matched a certain pattern?

To implement this functionality, we use BIP-32 HD keys with public derivation.

On initialization, a server picks a seed S and generates a root public key K
from it, and publishes K.

Users generate a transaction T and extract the CheckTemplateVerify hash H for
it. They then take H and convert it into a derivation path D of 8 u32's and 1
u8 for non-hardened derivation (see `hash_to_child_vec`).

This derivation path is then applied to K to generate a key C. This key is
added with a CheckSig(SIGHASH_ALL) to the script in place of a CTV clause.

Then, when a user desires to spend an output with such a key, they create the
entire transaction they want to occur and send it to the emulator server.

Without even checking to see that the key is used in the transaction, the
server generates the template hash H' (which should equal H) and then signs,
returning the signature to the client.

The implemented signing rule derives its key from this transaction's CTV hash
at input zero and signs with `SIGHASH_ALL`. The service does not accept an
arbitrary derivation path or a caller-selected signing policy. This restriction
is structural and does not depend on authenticating the requesting account.
Operator access controls can govern service availability and privacy; they do
not replace the covenant's key-derivation rule. Backend selection, key custody
and deployment availability still need explicit assumptions.

Before creating a contract, clients may wish to collect all possible
signatures required to prevent an availability fault.

This scheme has the benefit that:

1. contract specification can occur without any online processes
1. The server has no intelligent logic, all guarantees are structural.
1. Server is completely stateless.
1. Availability/malfeasance can be controlled for with multisig
1. 1:1 functionality mapping to CTV

The downside of this approach to emulation is that:

1. It is somewhat inefficient for scripts which have many branched possibilities.
1. No inherent mechanism to delete keys after use to protect against future exfiltration.


### Why BIP-32

We use BIP-32 because it is a well studied primitive and derivation paths are
compatible with existing signing hardware. While it is true that a tweak of
32 bytes could be directly applied to the key more efficiently, easier
interoperability with existing tools seemed to be the best path.
