# LayerTree with executed MID Script

This example implements the dense MID tier of [LayerTree](https://layertree.tech/)
as Sapio contracts and signed transactions. A shared Taproot output commits to
a Merkle-sum tree of member balances. Each member record includes its owner,
exit key, eviction authority, lease height, nonce and voucher sequence. Proofs
update individual records or aligned cohorts while conserving the committed
principal.

The implementation follows LayerTree v0.15.2 at
[`f64cc0e4d4471e32f7fd8294ecb0b1c7547c1166`](https://github.com/roasbeef/layertree/tree/f64cc0e4d4471e32f7fd8294ecb0b1c7547c1166).
The generated MID Script bytes match the pinned upstream vectors. The WASM
interpreter executes those scripts, including their proof, authorization,
amount, lease and successor-output checks. There is no LayerTree transition
predicate implemented in the host.

## Supported transitions

The fixed script set contains eleven branches at depths two through sixteen.
Depth one omits the three height-two branches. All trees are dense and
operator-positioned; total balances cannot exceed `2^31 - 1` satoshis.

| Branch | Behavior |
| --- | --- |
| `EXIT_1`, `EXIT_2` | One or two sequential owner-authorized exits, or eviction-authorized exits after each lease. Payouts use the committed exit key and exact amount. |
| `JOIN_0`, `JOIN_2` | Operator-authorized insertion into an empty leaf or aligned height-two subtree, funded by additional inputs. |
| `UPDATE` | Owner and operator authorize replacement of one member record. |
| `SPLIT` | The source owner signs a transaction carving a new member from its balance. |
| `CLAIM`, `CLAIM_H` | Redeem a recipient-bound voucher, optionally requiring its committed 32-byte preimage. |
| `EVICT_SUBTREE_1`, `EVICT_SUBTREE_2` | Sweep an aligned cohort after the required leases, paying every occupied member exactly. |
| `REWRITE_2` | Rewrite an aligned cohort under operator authorization and the reference script's owner-consent rules. Amount-only increases do not require the existing owner's signature. |

Every nonempty successor keeps the same physical script-tree commitment.
When the final member exits, the script permits closure without another funded
tree output. The runner reserves output zero as a zero-valued `OP_RETURN`,
retaining the prescribed payout indices. `compile_state` rejects empty states;
they are terminal transactions rather than funded contracts.

This profile does not implement sparse placement, `DETACH`, the optional
exit-consent or tower variants, pending-receive extensions, or the MIN/FULL/CCV
tiers. Lightning channels, Lightning network integration, operator networking
and wallet synchronization are outside this example.

## Binding the real Bitcoin outputs

Each native Taproot leaf is an ordinary signature check under the program key
for one exact MID Script and the distributed `bitcoin_script_mid.wasm` module.
The script's `m` witness is **the actual Merkle root of these oracle wrapper
leaves**. It is stable across states and commits to the complete executable
script set and selected oracle root.

The state root `r` determines the physical internal key:

```text
P = xonly(H + tagged_hash("LayerTree/state", r) * G)
output_key = TapTweak(P, m)
scriptPubKey = OP_1 PUSH32 output_key
```

`H` is BIP341's fixed NUMS point. A portable artifact retains its public scalar
opening; validation rederives the internal point and checks every declared
leaf. The compiler retains all oracle leaves and records only script-path
signing locations. It does not install a spendable owner or operator key path.

Consequently, the unchanged MID prologue's `TREE_SPK` calculation matches the
authenticated physical input script. Its successor checks match actual Bitcoin
outputs, without a mapping between hypothetical and physical addresses.
`OP_TX` reads the authenticated transaction projection. `CHECKSIG` uses the
authenticated native wrapper leaf's BIP341 signing context; vouchers use
`CHECKSIGFROMSTACK` over their explicit messages.

The MID guest supports CAT, the required stack and numeric instructions,
conditionals, SHA256/HASH256, CHECKSIG, CSV/CLTV, `OP_TX`, CSFS and TWEAKADD.
Its experimental opcode bytes and supported selectors are specified in
[the interpreter documentation](BITCOIN_SCRIPT_CAT.md#mid-execution-profile).
It accepts input zero plus at least one native witness-v0 funding input,
excludes annexes and independent Taproot covenant inputs, and retains the
10,000-byte Script, 520-byte element and 1,000-element stack bounds.

## Funding, vouchers and trust

The example separately funds admissions and miner fees. Its runner reserves
2,000 satoshis per transaction and round-trips both compiled artifacts and
spend intents before requesting signatures. Fee selection is local funding
policy; the covenant's balance and payout checks execute in Script.

A subtree graft commits to a supplied root and sum. It does not establish that
the operator supplied useful records or made them available. New joiners must
obtain and verify their records, inclusion proofs, balances and terms before
funding a graft. Existing members need their own current proofs to exercise
their exits.

Vouchers bind the lineage identifier, source body and claimed record; hashlocked
vouchers use a separate tag and payment hash. Carves advance the source sequence,
and the final sequence disables further claims. Clients remain responsible for
nonce history, lineage uniqueness, proof updates and voucher replay discipline.
In particular, reusing an owner/nonce identity after exit and readmission can
revive old authorizations; the in-memory tree only rejects current duplicates.
The example does not provide a persistent wallet or a global identity registry.

The emulation oracle must be honest and available. An owner exit does not need
the operator's signature, but it still needs the oracle to execute the script
and issue its wrapper signature. A dishonest oracle can bypass the pooled
members' authorization, payout and lease rules. Bitcoin validates the ordinary
wrapper signatures and transaction lock time; these experimental covenant
opcodes have not been activated by running this example.

## Running and checking

From the repository root with the pinned Rust toolchain:

```sh
cargo run --locked -p sapio_integration_tests --example layertree
cargo test --locked -p sapio-contrib --lib layertree
cargo test --locked -p sapio_integration_tests --test bitcoin_script_mid
cargo test --locked -p sapio_integration_tests --test layertree
bash evaluators/build.sh --check
```

The offline example uses published disposable keys and synthetic coins. It
prints linked, signed joins, a cohort graft and rewrite, an update, a split,
voucher claims, owner exits and leased cohort evictions. It does not fund or
broadcast those transactions. Unit tests compare Script bytes, records, roots,
proofs and output keys against upstream vectors, and check portable state
artifacts and the absence of program key-path authorization.

To fund and confirm the flow on an isolated regtest node:

```sh
cargo build --locked -p sapio_integration_tests --example layertree_vectors
python3 contrib/check_layertree.py /path/to/bitcoind /path/to/bitcoin-cli \
  target/debug/examples/layertree_vectors
```

The node validates ordinary funding and wrapper witnesses, while the oracle
executes the MID scripts before signing. Lease enforcement also uses Bitcoin's
transaction lock-time boundary; the node does not interpret the experimental
MID opcodes.

Sources: [contract and state compilation](../sapio-contrib/src/contracts/layertree.rs),
[tree records and proofs](../sapio-contrib/src/contracts/layertree/tree.rs),
[Script assembly](../sapio-contrib/src/contracts/layertree/script.rs),
[witness construction](../sapio-contrib/src/contracts/layertree/witness.rs), and
[signed runner](../integration_tests/src/layertree_example.rs).
