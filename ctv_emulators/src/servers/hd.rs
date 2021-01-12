use super::*;
#[derive(Clone)]
pub struct HDOracleEmulator {
    root: ExtendedPrivKey,
}

impl HDOracleEmulator {
    pub fn new(root: ExtendedPrivKey) -> Self {
        HDOracleEmulator { root }
    }
    pub async fn bind<A: ToSocketAddrs>(self, a: A) -> std::io::Result<()> {
        let listener = TcpListener::bind(a).await?;
        loop {
            let (mut socket, _) = listener.accept().await?;
            {
                let this = self.clone();
                let _: tokio::task::JoinHandle<Result<(), std::io::Error>> =
                    tokio::spawn(async move {
                        loop {
                            socket.readable().await?;
                            this.handle(&mut socket).await?;
                        }
                    });
            }
        }
    }
    fn derive(&self, h: Sha256, secp: &Secp256k1<All>) -> Result<ExtendedPrivKey, Error> {
        let c = hash_to_child_vec(h);
        self.root.derive_priv(secp, &c)
    }

    fn sign(
        &self,
        mut b: PartiallySignedTransaction,
        secp: &Secp256k1<All>,
    ) -> Result<PartiallySignedTransaction, std::io::Error> {
        let tx = b.clone().extract_tx();
        let h = tx.get_ctv_hash(0);
        if let Ok(key) = self.derive(h, secp) {
            if let Some(scriptcode) = &b.inputs[0].witness_script {
                if let Some(utxo) = &b.inputs[0].witness_utxo {
                    let mut sighash = bitcoin::util::bip143::SigHashCache::new(&tx);
                    let sighash = sighash.signature_hash(
                        0,
                        &scriptcode,
                        utxo.value,
                        bitcoin::blockdata::transaction::SigHashType::All,
                    );
                    let msg = bitcoin::secp256k1::Message::from_slice(&sighash[..])
                        .or_else(|_e| input_error("Message hash not valid (impossible?)"))?;
                    let mut signature: Vec<u8> = secp
                        .sign(&msg, &key.private_key.key)
                        .serialize_compact()
                        .into();
                    signature.push(0x01);
                    let pk = key.private_key.public_key(secp);
                    b.inputs[0].partial_sigs.insert(pk, signature);
                    return Ok(b);
                }
            }
        }
        input_error("Unknown Failure to Sign")
    }
    async fn handle(&self, t: &mut TcpStream) -> Result<(), std::io::Error> {
        let request = Self::requested(t).await?;
        match request {
            msgs::Request::SignPSBT(msgs::PSBT(unsigned)) => {
                let psbt = SECP.with(|secp| self.sign(unsigned, secp))?;
                Self::respond(t, &msgs::PSBT(psbt)).await
            }
            msgs::Request::ConfirmKey(msgs::ConfirmKey(_epk, s)) => {
                let ck = SECP.with(|secp| {
                    let key = self.root.private_key.key;
                    let entropy: [u8; 32] = rand::thread_rng().gen();
                    let h: Sha256 = Sha256::from_slice(&entropy).unwrap();
                    let mut m = Sha256::engine();
                    m.input(&h.into_inner());
                    m.input(&s.into_inner());
                    let msg = bitcoin::secp256k1::Message::from_slice(&Sha256::from_engine(m)[..])
                        .unwrap();
                    let signature = secp.sign(&msg, &key);
                    msgs::KeyConfirmed(signature, h)
                });
                Self::respond(t, &ck).await
            }
        }
    }

    async fn requested(t: &mut TcpStream) -> Result<msgs::Request, std::io::Error> {
        let l = t.read_u32().await? as usize;
        let mut v = vec![0u8; l];
        t.read_exact(&mut v[..]).await?;
        Ok(serde_json::from_slice(&v[..])?)
    }
    async fn respond<T: Serialize>(t: &mut TcpStream, r: &T) -> Result<(), std::io::Error> {
        let v = serde_json::to_vec(r)?;
        t.write_u32(v.len() as u32).await?;
        t.write_all(&v[..]).await?;
        t.flush().await
    }
}
