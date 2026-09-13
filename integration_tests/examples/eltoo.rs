//! Compile and finalize a disposable eltoo graph without broadcasting.

use bitcoin::consensus::encode::serialize_hex;
use bitcoin::{OutPoint, Transaction};
use emulator_connect::program::{ProgramOracle, ProgramSigningRequest};
use sapio::template::Template;
use sapio_contrib::contracts::eltoo::State;
use sapio_integration_tests::eltoo_example::fixture;
use sapio_integration_tests::eltoo_example::recovery::recover_update;
use sapio_integration_tests::eltoo_example::runner::{
    authorize_update, compile, finalize_candidate, settlement_request, settlement_template,
    sign_sponsor, update_request, update_template, Coin, Error,
};
use serde_json::json;

fn finish(
    oracle: &ProgramOracle,
    request: ProgramSigningRequest,
    template: &Template,
) -> Result<Transaction, Error> {
    let mut psbt = oracle.sign(request)?;
    sign_sponsor(&mut psbt, &fixture::sponsor_key())?;
    finalize_candidate(template, psbt)
}

fn output_zero(transaction: &Transaction) -> Coin {
    Coin {
        outpoint: OutPoint::new(transaction.compute_txid(), 0),
        txout: transaction.output[0].clone(),
    }
}

fn main() -> Result<(), Error> {
    let terms = fixture::terms();
    let oracle = ProgramOracle::new(fixture::oracle_key(), vec![])?;
    let joint = fixture::joint_key();
    let funding = terms.funding();
    let funding_coin = fixture::coin(&funding, 1);
    let first = State {
        number: 1,
        alice_sats: 60_000,
    };
    let latest = State {
        number: 3,
        alice_sats: 40_000,
    };
    let first_channel = terms.state(first)?;
    let latest_channel = terms.state(latest)?;
    let first_authorization = authorize_update(&terms, first, &joint)?;
    let latest_authorization = authorize_update(&terms, latest, &joint)?;
    let update_1 = finish(
        &oracle,
        update_request(
            &funding,
            first,
            funding_coin.clone(),
            fixture::sponsor(2_000, 2),
            &first_authorization,
        )?,
        &update_template(&terms, first)?,
    )?;
    let direct = update_request(
        &funding,
        latest,
        funding_coin,
        fixture::sponsor(2_000, 3),
        &latest_authorization,
    )?;
    let recovered = recover_update(&terms, &update_1)?;
    let rebound = recovered.request(
        &terms,
        latest,
        fixture::sponsor(3_000, 4),
        &latest_authorization,
    )?;
    assert_eq!(direct.witness, rebound.witness);
    let update_3_direct = finish(&oracle, direct, &update_template(&terms, latest)?)?;
    let update_3_rebound = finish(&oracle, rebound, &update_template(&terms, latest)?)?;
    let settlement = finish(
        &oracle,
        settlement_request(
            &latest_channel,
            output_zero(&update_3_rebound),
            fixture::sponsor(2_000, 5),
        )?,
        &settlement_template(&terms, latest)?,
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "research_only": true,
            "funding": "synthetic; no transactions broadcast",
            "joint_signing": "one disposable key simulates joint authorization; not MuSig2",
            "settlement_delay_blocks": terms.delay(),
            "artifacts": {
                "funding": compile(&funding)?,
                "state_1": compile(&first_channel)?,
                "state_3": compile(&latest_channel)?,
            },
            "authorizations": {
                "state_1": first_authorization.to_string(),
                "state_3_reused": latest_authorization.to_string(),
            },
            "transactions": {
                "update_1": serialize_hex(&update_1),
                "update_3_direct": serialize_hex(&update_3_direct),
                "update_3_rebound": serialize_hex(&update_3_rebound),
                "settlement_3": serialize_hex(&settlement),
            },
        }))?
    );
    Ok(())
}
