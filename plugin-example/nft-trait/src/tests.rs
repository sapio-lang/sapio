use super::*;
fn mint(royalty: f64) -> Mint_NFT_Trait_Version_0_1_0 {
    Mint_NFT_Trait_Version_0_1_0 {
        owner: "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
            .parse()
            .unwrap(),
        ipfs_nft: IpfsNFT {
            cid: "example".into(),
            version: 0,
            edition: 1,
            of_edition_count: 1,
            artist: None,
            blessing: None,
            softlink: None,
        },
        minting_module: None,
        royalty,
    }
}
#[test]
fn royalty_rejects_invalid_fraction_and_edition() {
    for royalty in [-0.1, 1.1, f64::INFINITY, f64::NAN] {
        assert!(mint(royalty)
            .compute_royalty_for_artist(Amount::from_sat(1000))
            .is_err());
    }
    let mut invalid = mint(0.0);
    invalid.ipfs_nft.edition = 0;
    assert!(invalid.validate().is_err());
}
#[test]
fn royalty_cannot_exceed_price_even_at_u64_boundary() {
    assert_eq!(
        mint(1.0)
            .compute_royalty_for_artist(Amount::from_sat(u64::MAX))
            .unwrap()
            .as_sat(),
        u64::MAX
    );
    assert_eq!(
        mint(0.0)
            .compute_royalty_for_artist(Amount::from_sat(u64::MAX))
            .unwrap(),
        Amount::ZERO
    );
    assert_eq!(
        mint(0.25)
            .compute_royalty_for_artist(Amount::from_sat(101))
            .unwrap()
            .as_sat(),
        25
    );
    assert_eq!(
        mint(0.000001)
            .compute_royalty_for_artist(Amount::from_sat(999999))
            .unwrap(),
        Amount::ZERO
    );
}
