use super::federated::FederatedEmulatorConnection;
use super::hd::HDOracleEmulatorConnection;
use crate::{msgs, wire, CTVEmulator, Clause, EmulatorError};
use bitcoin::hashes::sha256;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::secp256k1::{Keypair, Message, SecretKey};
use bitcoin::util::bip32::{ExtendedPrivKey, ExtendedPubKey};
use bitcoin::util::psbt::{raw, PartiallySignedTransaction as Psbt};
use bitcoin::util::taproot::{LeafVersion, TapLeafHash};
use bitcoin::{Network, Script, Transaction, TxIn, TxOut};
use bitcoin::{SchnorrSig, SchnorrSighashType};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;

type Mutation = fn(&mut Psbt);

fn request() -> Psbt {
    let mut psbt = Psbt::from_unsigned_tx(Transaction {
        version: 2,
        lock_time: 0,
        input: vec![TxIn::default()],
        output: vec![TxOut {
            value: 9_000,
            script_pubkey: Script::new(),
        }],
    })
    .unwrap();
    psbt.inputs[0].witness_utxo = Some(TxOut {
        value: 10_000,
        script_pubkey: Script::new(),
    });
    psbt.inputs[0].unknown.insert(
        raw::Key {
            type_value: 0x80,
            key: vec![1],
        },
        vec![2],
    );
    psbt
}

struct ChangeResponse(fn(&mut Psbt));

impl CTVEmulator for ChangeResponse {
    fn get_signer_for(&self, _: sha256::Hash) -> Result<Clause, EmulatorError> {
        Ok(Clause::Trivial)
    }

    fn sign(&self, mut psbt: Psbt) -> Result<Psbt, EmulatorError> {
        (self.0)(&mut psbt);
        Ok(psbt)
    }
}

struct ObserveResponse(Arc<AtomicUsize>);

impl CTVEmulator for ObserveResponse {
    fn get_signer_for(&self, _: sha256::Hash) -> Result<Clause, EmulatorError> {
        Ok(Clause::Trivial)
    }

    fn sign(&self, psbt: Psbt) -> Result<Psbt, EmulatorError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(psbt)
    }
}

#[test]
fn federation_rejects_changed_metadata_before_calling_another_participant() {
    let calls = Arc::new(AtomicUsize::new(0));
    let federation = FederatedEmulatorConnection::new(
        vec![
            Arc::new(ChangeResponse(|psbt| psbt.inputs[0].unknown.clear())),
            Arc::new(ObserveResponse(calls.clone())),
        ],
        2,
    );
    assert!(matches!(
        federation.sign(request()),
        Err(EmulatorError::InvalidResponse)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

fn add_signature(psbt: &mut Psbt, byte: u8) {
    let secp = Secp256k1::new();
    let keypair = Keypair::from_secret_key(&secp, &SecretKey::from_slice(&[byte; 32]).unwrap());
    psbt.inputs[0].tap_script_sigs.insert(
        (
            keypair.x_only_public_key().0,
            TapLeafHash::from_script(&Script::from(vec![0x51]), LeafVersion::TapScript),
        ),
        SchnorrSig {
            sig: secp.sign_schnorr_no_aux_rand(
                &Message::from_digest_slice(&[byte; 32]).unwrap(),
                &keypair,
            ),
            hash_ty: SchnorrSighashType::All,
        },
    );
}

#[test]
fn federation_accumulates_signatures_and_preserves_prior_participant_entries() {
    let original = request();
    let federation = FederatedEmulatorConnection::new(
        vec![
            Arc::new(ChangeResponse(|psbt| add_signature(psbt, 1))),
            Arc::new(ChangeResponse(|psbt| {
                assert_eq!(psbt.inputs[0].tap_script_sigs.len(), 1);
                add_signature(psbt, 2);
            })),
        ],
        2,
    );
    let response = federation.sign(original.clone()).unwrap();
    assert_eq!(response.inputs[0].tap_script_sigs.len(), 2);
    sapio_ctv_emulator_trait::validate_signing_response(&original, &response).unwrap();

    let calls = Arc::new(AtomicUsize::new(0));
    let invalid = FederatedEmulatorConnection::new(
        vec![
            Arc::new(ChangeResponse(|psbt| add_signature(psbt, 1))),
            Arc::new(ChangeResponse(|psbt| {
                psbt.inputs[0].tap_script_sigs.clear()
            })),
            Arc::new(ObserveResponse(calls.clone())),
        ],
        2,
    );
    assert!(matches!(
        invalid.sign(original),
        Err(EmulatorError::InvalidResponse)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hd_rejects_raw_metadata_changes_instead_of_hiding_them_in_a_merge() {
    let mutations: [(Mutation, bool); 5] = [
        (|psbt| psbt.inputs[0].unknown.clear(), false),
        (
            |psbt| psbt.inputs[0].witness_utxo.as_mut().unwrap().value -= 1,
            false,
        ),
        (
            |psbt| psbt.outputs[0].witness_script = Some(Script::from(vec![0x51])),
            false,
        ),
        (|_| {}, true),
        (|psbt| add_signature(psbt, 1), true),
    ];
    for (mutate, accepted) in mutations {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let msgs::Request::SignPSBT(msgs::PSBT(mut response)) =
                wire::read_message(&mut stream).await.unwrap();
            mutate(&mut response);
            wire::write_message(&mut stream, &msgs::PSBT(response))
                .await
                .unwrap();
        });
        let secp = Arc::new(Secp256k1::new());
        let root = ExtendedPubKey::from_priv(
            &secp,
            &ExtendedPrivKey::new_master(Network::Regtest, &[7; 32]).unwrap(),
        );
        let connection = HDOracleEmulatorConnection::new(address, root, None, secp)
            .await
            .unwrap();
        let response = connection.sign(request());
        peer.await.unwrap();
        if accepted {
            let mut expected = request();
            mutate(&mut expected);
            assert_eq!(response.unwrap(), expected);
        } else {
            assert!(matches!(response, Err(EmulatorError::InvalidResponse)));
        }
    }
}
