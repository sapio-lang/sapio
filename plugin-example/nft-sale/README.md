# Fixed NFT sale

Transfers the NFT after the sale height and splits the buyer price between owner and artist. Extra buyer funds are a separate input; the NFT value is preserved.

See the [workspace guide](../README.md) for Cargo build and test commands,
funding assumptions, and the complete executable catalog. This module has a
[representative input](../../contrib/vectors/examples/nft-sale.json).

Coverage: Catalog checks reminting, royalty outputs and lock time.
