# Vault with recovery

Provides hot/cold spending, delayed redemption and optional CPFP outputs.

See the [workspace guide](../README.md) for Cargo build and test commands,
funding assumptions, and the complete executable catalog. This module has a
[representative input](../../contrib/vectors/examples/jamesob-vault.json).

`spend_hot` and `spend_cold` treat the requested amount as the payment to the
destination. Both reserve the configured estimated fee and return any remainder
to a new vault in the **Secure** state. The hot path retains its relative delay;
spending its change with the hot key requires starting redemption again.

To withdraw without change, request the vault balance minus the estimated fee
for the single-output transaction. Requests that leave insufficient fees, or
change too small to construct another funded vault, fail instead of donating the
remainder. The retained fee cap also rejects an oversized funding UTXO at bind.

As with `backup` and `begin_redeem`, the fee estimate charges four weight units
per unsigned byte at `default_feerate` satoshis per 1,000 weight units. It does
not include satisfaction growth and does not guarantee a final relay feerate.

Coverage: Native overflow, rounding, compiled-funding, and hot/cold withdrawal
conservation and fee-boundary tests.
