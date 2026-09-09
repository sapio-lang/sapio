use super::*;
fn schedule(start: u64, minimum: u64, updates: u64) -> DutchAuctionData {
    DutchAuctionData {
        period: 6,
        start_price: start.into(),
        min_price: minimum.into(),
        updates,
    }
}
#[test]
fn price_schedule_includes_exact_endpoints_without_float_roundtrip() {
    let height = AbsHeight::try_from(100).unwrap();
    let values = schedule(101, 90, 3).create_schedule(height).unwrap();
    assert_eq!(
        values
            .iter()
            .map(|(h, p)| (h.get(), u64::from(*p)))
            .collect::<Vec<_>>(),
        vec![(100, 101), (106, 98), (112, 94), (118, 90)]
    );
    let values = schedule(u64::MAX, 0, 2).create_schedule(height).unwrap();
    assert_eq!(u64::from(values[2].1), 0);
}
#[test]
fn invalid_auction_schedule_is_rejected_before_allocation() {
    let height = AbsHeight::try_from(100).unwrap();
    for extra in [
        schedule(10, 20, 2),
        schedule(10, 0, 0),
        schedule(10, 0, 721),
        schedule(10, 0, u64::MAX),
    ] {
        assert!(extra.create_schedule(height).is_err());
    }
    let mut extra = schedule(10, 0, 1);
    extra.period = 0;
    assert!(extra.create_schedule(height).is_err());
    assert!(schedule(10, 0, 1)
        .create_schedule(AbsHeight::try_from(499999999).unwrap())
        .is_err());
}

#[test]
fn trait_extra_json_receives_the_same_checks_as_exact_data() {
    let mut main = NFT_Sale_Trait_Version_0_1_0 {
        sell_to: "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
            .parse()
            .unwrap(),
        price: 1000.into(),
        data: Mint_NFT_Trait_Version_0_1_0 {
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
            royalty: 0.0,
            minting_module: None,
        },
        sale_time: AbsHeight::try_from(100).unwrap(),
        extra: None,
    };
    for invalid in [
        schedule(10, 0, 0),
        schedule(10, 20, 2),
        schedule(10, 0, 721),
    ] {
        main.extra = Some(serde_json::to_string(&invalid).unwrap());
        assert!(
            NFTDutchAuction::try_from(Versions::NFT_Sale_Trait_Version_0_1_0(main.clone()))
                .is_err()
        );
    }
    main.extra = None;
    main.price = u64::MAX.into();
    assert!(NFTDutchAuction::try_from(Versions::NFT_Sale_Trait_Version_0_1_0(main)).is_err());
}
