use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::Hash;

use bitcoin::util::bip32::*;
use sapio::clause::Clause;
use sapio::contract::emulator::CTVEmulator;
use sapio::contract::error::CompilationError;
use std::io::Read;
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};

use bitcoin::consensus::encode::{Decodable, Encodable};
use bitcoin::secp256k1::{All, Secp256k1};
use bitcoin::util::psbt::PartiallySignedTransaction;
use sapio::template::CTVHash;
tokio::task_local! {
    pub static SECP: Secp256k1<All>;
}
#[derive(Clone)]
struct HDOracleEmulator {
    root: ExtendedPrivKey,
}
impl HDOracleEmulator {
    async fn bind<A: ToSocketAddrs>(self, a: A) -> std::io::Result<()> {
        let listener = TcpListener::bind(a).await?;
        SECP.scope(Secp256k1::new(), async {
            loop {
                let (mut socket, _) = listener.accept().await?;
                {
                    let this = self.clone();
                    tokio::spawn(async move {
                        loop {
                            this.handle(&mut socket).await;
                        }
                    });
                }
            }
            Ok(())
        })
        .await
    }
    fn derive(&self, h: Sha256, secp: &Secp256k1<All>) -> Result<ExtendedPrivKey, Error> {
        let a: [u8; 32] = h.into_inner();
        let b: [[u8; 4]; 8] = unsafe { std::mem::transmute(a) };
        let c: Vec<ChildNumber> = b
            .iter()
            .map(|x| u32::from_be_bytes(*x))
            .map(ChildNumber::from)
            .collect();
        self.root.derive_priv(secp, &c)
    }

    fn sign(
        &self,
        mut b: bitcoin::util::psbt::PartiallySignedTransaction,
        secp: &Secp256k1<All>,
    ) -> bitcoin::util::psbt::PartiallySignedTransaction {
        let tx = b.clone().extract_tx();
        let h = tx.get_ctv_hash(0);
        if let Ok(key) = self.derive(h, secp) {
            let pk = key.private_key.public_key(secp);
            let sighash = bitcoin::util::bip143::SighashComponents::new(&tx);

            if let Some(scriptcode) = &b.inputs[0].witness_script {
                if let Some(utxo) = &b.inputs[0].witness_utxo {
                    let sighash = sighash.sighash_all(&tx.input[0], &scriptcode, utxo.value);
                    let msg = bitcoin::secp256k1::Message::from_slice(&sighash[..]).unwrap();
                    let mut signature: Vec<u8> = secp
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
    const MAX_MSG: usize = 1_000_000;
    async fn handle(&self, t: &mut TcpStream) -> Result<(), std::io::Error> {
        let len = t.read_u32().await? as usize;
        if len > Self::MAX_MSG {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Invalid Length",
            ));
        }
        let mut m = vec![0; len];
        let read = t.read_exact(&mut m[..]).await?;
        if read != len {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Invalid Length",
            ));
        }

        let psbt: PartiallySignedTransaction = Decodable::consensus_decode(&m[..]).unwrap();
        m.clear();
        let b = SECP.with(|secp| self.sign(psbt, secp));
        b.consensus_encode(&mut m);
        t.write_u32(m.len() as u32).await?;
        t.write_all(&m[..]).await?;
        Ok(())
    }
}
struct HDOracleEmulatorConnection {
    address: SocketAddr,
    connection: Mutex<TcpStream>,
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
use tokio::sync::Mutex;
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
        let mut out = vec![];
        b.consensus_encode(&mut out);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let res: Result<PartiallySignedTransaction, _> = rt.block_on(async {
            let mut conn = self.connection.lock().await;
            conn.write_u32(out.len() as u32).await?;
            conn.write_all(&out[..]).await?;
            let len = conn.read_u32().await? as usize;
            let mut inp = vec![0; len];
            if len != conn.read_exact(&mut inp[..]).await? {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Invalid Length",
                ));
            }
            Decodable::consensus_decode(&inp[..])
                .map_err(|_e| std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid PSBT"))
        });
        res.unwrap_or_else(|_e| b)
    }
}
