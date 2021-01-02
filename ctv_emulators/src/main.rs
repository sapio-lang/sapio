use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::Hash;

use bitcoin::util::bip32::*;
use sapio::clause::Clause;
use sapio::contract::emulator::CTVEmulator;
use sapio::contract::error::CompilationError;

use sapio::template::CTVHash;
struct HDOracleEmulator {
    root: ExtendedPrivKey,
    secp: Box<bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::All>>,
}
impl HDOracleEmulator {
    fn derive(&self, h: Sha256) -> Result<ExtendedPrivKey, Error> {
        let a: [u8; 32] = h.into_inner();
        let b: [[u8; 4]; 8] = unsafe { std::mem::transmute(a) };
        let c: Vec<ChildNumber> = b
            .iter()
            .map(|x| u32::from_be_bytes(*x))
            .map(ChildNumber::from)
            .collect();
        self.root.derive_priv(&self.secp, &c)
    }

    fn sign(
        &self,
        mut b: bitcoin::util::psbt::PartiallySignedTransaction,
    ) -> bitcoin::util::psbt::PartiallySignedTransaction {
        let tx = b.clone().extract_tx();
        let h = tx.get_ctv_hash(0);
        if let Ok(key) = self.derive(h) {
            let pk = key.private_key.public_key(&self.secp);
            let sighash = bitcoin::util::bip143::SighashComponents::new(&tx);

            if let Some(scriptcode) = &b.inputs[0].witness_script {
                if let Some(utxo) = &b.inputs[0].witness_utxo {
                    let sighash = sighash.sighash_all(&tx.input[0], &scriptcode, utxo.value);
                    let msg = bitcoin::secp256k1::Message::from_slice(&sighash[..]).unwrap();
                    let mut signature: Vec<u8> = self
                        .secp
                        .sign(&msg, &key.private_key.key)
                        .serialize_compact()
                        .into();
                    signature.push(0x01);
                    b.inputs[0].partial_sigs.insert(pk, signature);
                    return b;
                }
            }
        }
        b
    }
}
struct HDOracleEmulatorConnection {
    root: ExtendedPubKey,
    secp: Box<bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::All>>,
}

impl HDOracleEmulatorConnection {
    fn derive(&self, h: Sha256) -> Result<ExtendedPubKey, Error> {
        let a: [u8; 32] = h.into_inner();
        let b: [[u8; 4]; 8] = unsafe { std::mem::transmute(a) };
        let c: Vec<ChildNumber> = b
            .iter()
            .map(|x| u32::from_be_bytes(*x))
            .map(ChildNumber::from)
            .collect();
        self.root.derive_pub(&self.secp, &c)
    }
}
impl CTVEmulator for HDOracleEmulatorConnection {
    fn get_signer_for(
        &self,
        h: Sha256,
    ) -> Result<sapio::clause::Clause, sapio::contract::error::CompilationError> {
        Ok(Clause::Key(
            self.derive(h).map_err(CompilationError::custom)?.public_key,
        ))
    }
    fn sign(
        &self,
        b: bitcoin::util::psbt::PartiallySignedTransaction,
    ) -> bitcoin::util::psbt::PartiallySignedTransaction {
        b
    }
}
fn main() {
    println!("Hello, world!");
}
