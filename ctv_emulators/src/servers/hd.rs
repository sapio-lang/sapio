// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! definitions for oracle servers
use super::*;
use bitcoin::util::sighash::Prevouts;
use bitcoin::util::taproot::TapLeafHash;
use bitcoin::util::taproot::TapSighashHash;
use bitcoin::SchnorrSig;
use bitcoin::Script;
use bitcoin::TxOut;
use bitcoin::XOnlyPublicKey;

/// hierarchical deterministic oracle emulator
#[derive(Clone)]
pub struct HDOracleEmulator {
    root: ExtendedPrivKey,
    debug: bool,
}

impl HDOracleEmulator {
    /// create a new HDOracleEmulator
    ///
    /// if debug is set, runs in a "single threaded" mode where we can observe errors on connections rather than ignoring them.
    pub fn new(root: ExtendedPrivKey, debug: bool) -> Self {
        HDOracleEmulator { root, debug }
    }
    /// binds a HDOracleEmulator to a socket interface and runs the server
    ///
    /// This will only return when debug = false if The TcpListener fails.
    /// When debug = true, then we join each connection one at a time and return
    /// any errors.
    pub async fn bind<A: ToSocketAddrs>(self, a: A) -> std::io::Result<()> {
        let listener = TcpListener::bind(a).await?;
        loop {
            let (mut socket, _) = listener.accept().await?;
            {
                let this = self.clone();
                let j: tokio::task::JoinHandle<Result<(), std::io::Error>> =
                    tokio::spawn(async move {
                        loop {
                            socket.readable().await?;
                            this.handle(&mut socket).await?;
                        }
                    });
                if self.debug {
                    tokio::join!(j).0??;
                }
            }
        }
    }
    /// helper to get an EPK for the oracle.
    fn derive(&self, h: Sha256, secp: &Secp256k1<All>) -> Result<ExtendedPrivKey, Error> {
        let c = hash_to_child_vec(h);
        self.root.derive_priv(secp, &c)
    }

    /// Signs a PSBT with the correct derived key.
    ///
    /// Always signs for spending index 0.
    ///
    /// May fail to sign if the PSBT is not properly formatted
    fn sign(
        &self,
        mut b: PartiallySignedTransaction,
        secp: &Secp256k1<All>,
    ) -> Result<PartiallySignedTransaction, std::io::Error> {
        let tx = b.clone().extract_tx();
        let h = tx.get_ctv_hash(0);
        let utxos: Vec<TxOut> = b
            .inputs
            .iter()
            .map(|o| o.witness_utxo.clone())
            .collect::<Option<Vec<TxOut>>>()
            .ok_or_else(|| input_err("Could not find one of the UTXOs to be signed over"))?;
        let key = self
            .derive(h, secp)
            .map_err(|_| input_err("Could Not Derive Key"))?;
        let untweaked = key.to_keypair(secp);
        let pk = XOnlyPublicKey::from_keypair(&untweaked);
        let mut sighash = bitcoin::util::sighash::SighashCache::new(&tx);
        let input_zero = &mut b.inputs[0];
        use bitcoin::schnorr::TapTweak;
        let tweaked = untweaked
            .tap_tweak(secp, input_zero.tap_merkle_root)
            .into_inner();
        let tweaked_pk = tweaked.public_key();
        let hash_ty = bitcoin::util::sighash::SchnorrSighashType::All;
        let prevouts = &Prevouts::All(&utxos);
        let mut get_sig = |path, kp| {
            let annex = None;
            let sighash: TapSighashHash = sighash
                .taproot_signature_hash(0, prevouts, annex, path, hash_ty)
                .expect("Signature hash cannot fail...");
            let msg = bitcoin::secp256k1::Message::from_slice(&sighash[..])
                .expect("Size must be correct.");
            let sig = secp.sign_schnorr_no_aux_rand(&msg, kp);
            SchnorrSig { sig, hash_ty }
        };
        if let Some(true) = input_zero.witness_utxo.as_ref().map(|v| {
            v.script_pubkey
                == Script::new_v1_p2tr_tweaked(
                    XOnlyPublicKey::from(tweaked_pk).dangerous_assume_tweaked(),
                )
        }) {
            let sig = get_sig(None, &tweaked);
            input_zero.tap_key_sig = Some(sig);
        }
        for tlh in input_zero
            .tap_scripts
            .values()
            .map(|(script, ver)| TapLeafHash::from_script(script, *ver))
        {
            let sig = get_sig(Some((tlh, 0xffffffff)), &untweaked);
            input_zero.tap_script_sigs.insert((pk.0, tlh), sig);
        }
        Ok(b)
    }

    /// the main server business logic.
    ///
    /// - on receiving Request::SignPSBT, signs the PSBT.
    async fn handle(&self, t: &mut TcpStream) -> Result<(), std::io::Error> {
        let request = Self::requested(t).await?;
        match request {
            msgs::Request::SignPSBT(msgs::PSBT(unsigned)) => {
                let psbt = SECP.with(|secp| self.sign(unsigned, secp))?;
                Self::respond(t, &msgs::PSBT(psbt)).await
            }
        }
    }

    /// receive a request via the tcpstream.
    /// wire format: length:u32 data:[u8;length]
    ///
    /// TODO: DoS Critical: limit the allowed max length we will attempt to derserialize
    async fn requested(t: &mut TcpStream) -> Result<msgs::Request, std::io::Error> {
        let l = t.read_u32().await? as usize;
        let mut v = vec![0u8; l];
        t.read_exact(&mut v[..]).await?;
        Ok(serde_json::from_slice(&v[..])?)
    }

    /// respond via the tcpstream.
    /// wire format: length:u32 data:[u8;length]
    async fn respond<T: Serialize>(t: &mut TcpStream, r: &T) -> Result<(), std::io::Error> {
        let v = serde_json::to_vec(r)?;
        t.write_u32(v.len() as u32).await?;
        t.write_all(&v[..]).await?;
        t.flush().await
    }
}
