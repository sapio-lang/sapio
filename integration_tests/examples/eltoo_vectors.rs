//! Emit a complete eltoo contest scenario for the isolated Core driver.

use bitcoin::consensus::encode::serialize_hex;
use bitcoin::secp256k1::{schnorr::Signature, Secp256k1};
use bitcoin::taproot::{LeafVersion, TapLeafHash};
use bitcoin::{Address, Network, OutPoint, Transaction, Witness};
use emulator_connect::program::{
    prepare_program_request, ProgramOracle, ProgramSigningRequest, ProgramSpendPath,
};
use sapio_contrib::contracts::eltoo::{Channel, State};
use sapio_integration_tests::eltoo_example::fixture;
use sapio_integration_tests::eltoo_example::recovery::recover_update;
use sapio_integration_tests::eltoo_example::runner::{
    attach_inputs, authorize_update, compile, input, settlement_request, sign_sponsor, update_leaf,
    update_request, update_requirement, update_transaction, Coin, Error, Sponsor,
};

const SPONSOR_FEE: u64 = 2_000;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct Funding {
    funding: OutPoint,
    sponsors: Vec<OutPoint>,
}

fn finish(oracle: &ProgramOracle, request: ProgramSigningRequest) -> Result<Transaction, Error> {
    let mut signed = oracle.sign(request)?;
    sign_sponsor(&mut signed, &fixture::sponsor_key())?;
    Ok(sapio_psbt::finalize::finalize(signed, &Secp256k1::new())
        .map_err(|(_, errors)| format!("eltoo vector finalization failed: {errors:?}"))?
        .extract_tx()?)
}

fn output_zero(transaction: &Transaction) -> Coin {
    Coin {
        outpoint: OutPoint::new(transaction.compute_txid(), 0),
        txout: transaction.output[0].clone(),
    }
}

// Deliberately assemble a counterexample which the normal finalizer rejects.
// Both signatures are authentic; only the source state's native CLTV fails.
fn non_advancing_update(
    oracle: &ProgramOracle,
    source: &Channel,
    target: State,
    coin: Coin,
    sponsor: Sponsor,
    authorization: &Signature,
) -> Result<Transaction, Error> {
    let script = update_leaf(source)?;
    let leaf = TapLeafHash::from_script(&script, LeafVersion::TapScript);
    let input = input(source, &coin)?;
    let compiled = compile(source)?;
    let requirement = update_requirement(&compiled)?;
    assert_eq!(requirement.path, ProgramSpendPath::ScriptPath(leaf));
    let request = prepare_program_request(
        &compiled,
        &requirement,
        attach_inputs(
            update_transaction(source.terms(), target)?,
            coin,
            input,
            sponsor,
        )?,
        0,
        authorization.as_ref().to_vec(),
    )?;
    let mut signed = oracle.sign(request)?;
    sign_sponsor(&mut signed, &fixture::sponsor_key())?;
    assert!(sapio_psbt::finalize::finalize(signed.clone(), &Secp256k1::new()).is_err());
    let key = requirement.program.derive_public_key()?;
    let signature = signed.inputs[0]
        .tap_script_sigs
        .get(&(key, leaf))
        .ok_or("missing update signature")?;
    let control = signed.inputs[0]
        .tap_scripts
        .iter()
        .find(|(_, (candidate, version))| TapLeafHash::from_script(candidate, *version) == leaf)
        .map(|(control, _)| control)
        .ok_or("missing update proof")?;
    let channel_witness =
        Witness::from_slice(&[signature.to_vec(), script.to_bytes(), control.serialize()]);
    let sponsor_witness = Witness::from_slice(&[signed.inputs[1]
        .tap_key_sig
        .ok_or("missing sponsor signature")?
        .to_vec()]);
    let mut transaction = signed.unsigned_tx;
    transaction.input[0].witness = channel_witness;
    transaction.input[1].witness = sponsor_witness;
    Ok(transaction)
}

fn main() -> Result<(), Error> {
    let supplied: Option<Funding> = std::env::args_os()
        .nth(1)
        .map(|path| {
            std::fs::read(path)
                .map_err(Error::from)
                .and_then(|bytes| serde_json::from_slice(&bytes).map_err(Error::from))
        })
        .transpose()?;
    if supplied
        .as_ref()
        .is_some_and(|funding| funding.sponsors.len() != 3)
    {
        return Err("the contest scenario needs exactly three sponsor outpoints".into());
    }
    let terms = fixture::terms();
    let oracle = ProgramOracle::new(fixture::oracle_key(), vec![])?;
    let funding = terms.funding();
    let mut funding_coin = fixture::coin(&funding, 1);
    if let Some(supplied) = &supplied {
        funding_coin.outpoint = supplied.funding;
    }
    let sponsors: Vec<_> = (0..3)
        .map(|index| {
            let mut sponsor = fixture::sponsor(SPONSOR_FEE, index as u8 + 2);
            if let Some(supplied) = &supplied {
                sponsor.coin.outpoint = supplied.sponsors[index];
            }
            sponsor
        })
        .collect();
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
    let certificate_1 = authorize_update(&terms, first, &fixture::joint_key())?;
    let certificate_3 = authorize_update(&terms, latest, &fixture::joint_key())?;
    let update_1 = finish(
        &oracle,
        update_request(
            &funding,
            first,
            funding_coin.clone(),
            sponsors[0].clone(),
            &certificate_1,
        )?,
    )?;
    let direct = update_request(
        &funding,
        latest,
        funding_coin.clone(),
        sponsors[1].clone(),
        &certificate_3,
    )?;
    let recovered = recover_update(&terms, &update_1)?;
    let rebound = recovered.request(&terms, latest, sponsors[1].clone(), &certificate_3)?;
    assert_eq!(direct.witness, rebound.witness);
    let update_3_direct = finish(&oracle, direct)?;
    let update_3_rebound = finish(&oracle, rebound)?;
    let settlement_1 = finish(
        &oracle,
        settlement_request(&first_channel, output_zero(&update_1), sponsors[2].clone())?,
    )?;
    let settlement_3 = finish(
        &oracle,
        settlement_request(
            &latest_channel,
            output_zero(&update_3_rebound),
            sponsors[2].clone(),
        )?,
    )?;
    let equal = non_advancing_update(
        &oracle,
        &first_channel,
        first,
        output_zero(&update_1),
        sponsors[1].clone(),
        &certificate_1,
    )?;
    let older = non_advancing_update(
        &oracle,
        &latest_channel,
        first,
        output_zero(&update_3_rebound),
        sponsors[2].clone(),
        &certificate_1,
    )?;
    let funding_address =
        Address::from_script(&funding_coin.txout.script_pubkey, Network::Regtest)?;
    let sponsor_address =
        Address::from_script(&sponsors[0].coin.txout.script_pubkey, Network::Regtest)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "funding": {"address": funding_address.to_string(), "amount_sats": terms.capacity()},
            "sponsor": {"address": sponsor_address.to_string(), "amount_sats": SPONSOR_FEE, "count": 3},
            "delay_blocks": terms.delay(),
            "payouts": [
                {"address": terms.alice().to_string(), "amount_sats": latest.alice_sats},
                {"address": terms.bob().to_string(), "amount_sats": terms.capacity() - latest.alice_sats},
            ],
            "authorizations": {"direct": certificate_3.to_string(), "rebound": certificate_3.to_string()},
            "transactions": {
                "update_1": serialize_hex(&update_1),
                "update_3_direct": serialize_hex(&update_3_direct),
                "update_3_rebound": serialize_hex(&update_3_rebound),
                "settlement_1": serialize_hex(&settlement_1),
                "settlement_3": serialize_hex(&settlement_3),
                "equal_update_from_1": serialize_hex(&equal),
                "older_update_from_3": serialize_hex(&older),
            },
        }))?
    );
    Ok(())
}
