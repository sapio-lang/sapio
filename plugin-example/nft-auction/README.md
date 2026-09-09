# Dutch NFT auction

Provides owner-signed sales from the starting price through the exact minimum. Schedules allow 1–720 decreases and a positive block period; the default spans 4,320 blocks at six-block intervals.

See the [workspace guide](../README.md) for Cargo build and test commands,
funding assumptions, and the complete executable catalog. This module has a
[representative input](../../contrib/vectors/examples/nft-auction.json).

Coverage: Native endpoint, overflow and all-constructor validation; catalog checks three prices.
