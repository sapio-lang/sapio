//! Recover an update proof without retaining an old state's payout allocation.
//!
//! This recovery path has chain data and public channel terms, but no compiled
//! artifact for the old state. It reconstructs and authenticates the published
//! spending proof before making a low-level request. Ordinary exported-contract
//! requests use the artifact's recorded program requirements instead.

use super::runner::{attach_inputs, update_template, Coin, Error, Sponsor};
use bitcoin::psbt::Input;
use bitcoin::secp256k1::{schnorr::Signature, Secp256k1};
use bitcoin::taproot::{ControlBlock, LeafVersion, TapLeafHash};
use bitcoin::{OutPoint, Transaction, XOnlyPublicKey};
use emulator_connect::program::{ProgramSigningRequest, ProgramSpendPath, PSBT};
use sapio_contrib::contracts::eltoo::{State, Terms, LOCK_TIME_BASE, RECOVERY_TAG};

/// Public spending data reconstructed from a published update and fixed terms.
pub struct RecoveredUpdate {
    pub state_number: u32,
    pub coin: Coin,
    pub input: Input,
    pub leaf_hash: TapLeafHash,
}

impl RecoveredUpdate {
    /// Rebind the latest auxiliary authorization, retaining the verified proof.
    pub fn request(
        &self,
        terms: &Terms,
        target: State,
        sponsor: Sponsor,
        authorization: &Signature,
    ) -> Result<ProgramSigningRequest, Error> {
        if target.number <= self.state_number {
            return Err("a recovered update must advance the state number".into());
        }
        let template = update_template(terms, target)?;
        let psbt = attach_inputs(
            template.tx.clone(),
            self.coin.clone(),
            self.input.clone(),
            sponsor,
        )?;
        template.check_funded_psbt(&psbt)?;
        Ok(ProgramSigningRequest {
            instance: terms.update_program().instance().clone(),
            input_index: 0,
            witness: authorization.as_ref().to_vec(),
            path: ProgramSpendPath::ScriptPath(self.leaf_hash),
            psbt: PSBT(psbt),
        })
    }
}

/// Verify a published sibling against the actual state output, then recover its
/// update control block. The caller obtains `observed` from its validated chain;
/// this helper establishes the spending proof, not transaction inclusion.
pub fn recover_update(terms: &Terms, observed: &Transaction) -> Result<RecoveredUpdate, Error> {
    if observed.version != bitcoin::transaction::Version::TWO
        || observed.input.len() != 2
        || observed
            .input
            .iter()
            .any(|input| input.sequence != bitcoin::Sequence::ZERO || !input.script_sig.is_empty())
        || observed.output.len() != 2
        || observed.output[0].value.to_sat() != terms.capacity()
        || observed.output[1].value.to_sat() != 0
    {
        return Err("observed update does not match the channel transaction format".into());
    }
    let number = observed
        .lock_time
        .to_consensus_u32()
        .checked_sub(LOCK_TIME_BASE)
        .ok_or("invalid state timestamp")?;
    terms.lock_time(number)?;
    if number == terms.max_state() {
        return Err("the maximum state has no update path to recover".into());
    }
    let publication = observed.output[1].script_pubkey.as_bytes();
    if publication.len() != 42
        || publication[..2] != [0x6a, 40]
        || &publication[2..10] != RECOVERY_TAG
    {
        return Err("expected one canonical tagged settlement-leaf publication".into());
    }
    let output = &observed.output[0];
    if !output.script_pubkey.is_p2tr() {
        return Err("observed state must be a native P2TR output".into());
    }
    let output_key = XOnlyPublicKey::from_slice(&output.script_pubkey.as_bytes()[2..])?;
    let script = terms.update_script(number)?;
    let leaf_hash = TapLeafHash::from_script(&script, LeafVersion::TapScript);
    // Both remaining branches have depth one. Recover parity by checking the
    // actual output commitment; no private key or old descriptor is needed.
    let mut encoded = Vec::with_capacity(65);
    encoded.push(0xc0);
    encoded.extend_from_slice(&terms.joint_key().serialize());
    encoded.extend_from_slice(&publication[10..]);
    let secp = Secp256k1::verification_only();
    for parity in 0..=1 {
        encoded[0] = 0xc0 | parity;
        let control = ControlBlock::decode(&encoded)?;
        if control.verify_taproot_commitment(&secp, output_key, &script) {
            return Ok(RecoveredUpdate {
                state_number: number,
                coin: Coin {
                    outpoint: OutPoint::new(observed.compute_txid(), 0),
                    txout: output.clone(),
                },
                input: Input {
                    witness_utxo: Some(output.clone()),
                    tap_internal_key: Some(terms.joint_key()),
                    tap_scripts: [(control, (script, LeafVersion::TapScript))].into(),
                    ..Input::default()
                },
                leaf_hash,
            });
        }
    }
    Err("published settlement leaf does not authenticate this state output".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eltoo_example::{fixture, runner::update_transaction};
    use bitcoin::ScriptBuf;

    #[test]
    fn recovery_authenticates_the_counter_sibling_and_actual_output() {
        let terms = fixture::terms();
        let observed = update_transaction(
            &terms,
            State {
                number: 1,
                alice_sats: 12_345,
            },
        )
        .unwrap();
        let recovered = recover_update(&terms, &observed).unwrap();
        assert_eq!(recovered.state_number, 1);
        assert_eq!(
            recovered.coin.outpoint,
            OutPoint::new(observed.compute_txid(), 0)
        );
        assert_eq!(recovered.input.tap_scripts.len(), 1);
        assert_eq!(recovered.input.tap_internal_key, Some(terms.joint_key()));

        let mut wrong_counter = observed.clone();
        wrong_counter.lock_time = bitcoin::absolute::LockTime::from_consensus(
            wrong_counter.lock_time.to_consensus_u32() + 1,
        );
        assert!(recover_update(&terms, &wrong_counter).is_err());
        for byte in [2, 10, 41] {
            let mut wrong_publication = observed.clone();
            let mut script = wrong_publication.output[1].script_pubkey.to_bytes();
            script[byte] ^= 1;
            wrong_publication.output[1].script_pubkey = ScriptBuf::from(script);
            assert!(recover_update(&terms, &wrong_publication).is_err());
        }
        let other = update_transaction(
            &terms,
            State {
                number: 1,
                alice_sats: 54_321,
            },
        )
        .unwrap();
        let mut wrong_output = observed.clone();
        wrong_output.output[0] = other.output[0].clone();
        assert!(recover_update(&terms, &wrong_output).is_err());
        let mut wrong_amount = observed.clone();
        wrong_amount.output[0].value -= bitcoin::Amount::ONE_SAT;
        assert!(recover_update(&terms, &wrong_amount).is_err());
        let mut trailing = observed;
        let mut script = trailing.output[1].script_pubkey.to_bytes();
        script.push(0);
        trailing.output[1].script_pubkey = ScriptBuf::from(script);
        assert!(recover_update(&terms, &trailing).is_err());
    }
}
