use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::Hash;
use bitcoin::util::bip32::*;
pub mod emulator;
use emulator::Clause;
pub use emulator::{CTVEmulator, EmulatorError, NullEmulator};

use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};

use bitcoin::consensus::encode::{Decodable, Encodable};
use bitcoin::secp256k1::{All, Secp256k1};
use bitcoin::util::psbt::PartiallySignedTransaction;
use sapio_base::CTVHash;
use std::sync::Arc;
const MAX_MSG: usize = 1_000_000;

thread_local! {
    pub static SECP: Secp256k1<All> = Secp256k1::new();
}

fn input_error<T>(s: &str) -> Result<T, std::io::Error> {
    Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, s))
}

fn hash_to_child_vec(h: Sha256) -> Vec<ChildNumber> {
    let a: [u8; 32] = h.into_inner();
    let b: [[u8; 4]; 8] = unsafe { std::mem::transmute(a) };
    let mut c: Vec<ChildNumber> = b
        .iter()
        // Note: We mask off the top bit. This removes 8 bits of entropy from the hash,
        // but we add it back in later.
        .map(|x| (u32::from_be_bytes(*x) << 1) >> 1)
        .map(ChildNumber::from)
        .collect();
    // Add a unique 9th path for the MSB's
    c.push(
        b.iter()
            .enumerate()
            .map(|(i, x)| (u32::from_be_bytes(*x) >> 31) << i)
            .sum::<u32>()
            .into(),
    );
    c
}
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
            let pk = key.private_key.public_key(secp);
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
                    b.inputs[0].partial_sigs.insert(pk, signature);
                    return Ok(b);
                }
            }
        }
        input_error("Unknown Failure to Sign")
    }
    async fn handle(&self, t: &mut TcpStream) -> Result<(), std::io::Error> {
        let len = t.read_u32().await? as usize;
        if len > MAX_MSG {
            return input_error("Invalid Length");
        }
        let mut m = vec![0; len];
        if t.read_exact(&mut m[..]).await? == len {
            let psbt: PartiallySignedTransaction = {
                let unsigned = Decodable::consensus_decode(&m[..])
                    .or_else(|_e| input_error("Invalid PSBT Received"))?;
                SECP.with(|secp| self.sign(unsigned, secp))?
            };
            // clear so we can reuse it!
            m.clear();
            psbt.consensus_encode(&mut m)
                .or_else(|_e| input_error("Invalid PSBT After Signing"))?;
            t.write_u32(m.len() as u32).await?;
            t.write_all(&m[..]).await?;
            t.flush().await?;
            Ok(())
        } else {
            input_error("Invalid Length")
        }
    }
}
pub struct HDOracleEmulatorConnection {
    runtime: Arc<tokio::runtime::Runtime>,
    connection: Mutex<Option<TcpStream>>,
    reconnect: SocketAddr,
    root: ExtendedPubKey,
    secp: Arc<bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::All>>,
}

impl HDOracleEmulatorConnection {
    fn derive(&self, h: Sha256) -> Result<ExtendedPubKey, Error> {
        let c = hash_to_child_vec(h);
        self.root.derive_pub(&self.secp, &c)
    }
    pub async fn new<A: ToSocketAddrs + std::fmt::Display + Clone>(
        address: A,
        root: ExtendedPubKey,
        runtime: Arc<tokio::runtime::Runtime>,
        secp: Arc<bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::All>>,
    ) -> Result<Self, std::io::Error> {
        Ok(HDOracleEmulatorConnection {
            connection: Mutex::new(None),
            reconnect: tokio::net::lookup_host(address.clone())
                .await?
                .next()
                .ok_or_else(|| {
                    input_error::<()>(&format!("Bad Lookup Could Not Resolve Address {}", address))
                        .unwrap_err()
                })?,
            runtime,
            root,
            secp,
        })
    }
}
use tokio::sync::Mutex;
impl CTVEmulator for HDOracleEmulatorConnection {
    fn get_signer_for(&self, h: Sha256) -> Result<Clause, EmulatorError> {
        Ok(Clause::Key(self.derive(h)?.public_key))
    }
    fn sign(
        &self,
        mut b: PartiallySignedTransaction,
    ) -> Result<PartiallySignedTransaction, EmulatorError> {
        let mut out = vec![];
        b.consensus_encode(&mut out)
            .or_else(|_e| input_error("Could not encode provided PSBT"))?;
        let inp = self.runtime.block_on(async {
            let mut mconn = self.connection.lock().await;
            loop {
                if let Some(conn) = &mut *mconn {
                    conn.write_u32(out.len() as u32).await?;
                    conn.write_all(&out[..]).await?;
                    conn.flush().await?;
                    let len = conn.read_u32().await? as usize;
                    if len > MAX_MSG {
                        return input_error("Invalid Length");
                    }
                    let mut inp = vec![0; len];
                    if len == conn.read_exact(&mut inp[..]).await? {
                        return Ok(inp);
                    } else {
                        return input_error("Invalid Length");
                    }
                } else {
                    *mconn = Some(TcpStream::connect(&self.reconnect).await?);
                }
            }
        })?;

        b.merge(Decodable::consensus_decode(&inp[..]).or_else(|_e| input_error("Invalid PSBT"))?)
            .or_else(|_e| input_error("Fault Signed PSBT"))?;
        Ok(b)
    }
}

pub struct FederatedEmulatorConnection {
    emulators: Vec<Box<dyn CTVEmulator>>,
    threshold: u8,
}

impl FederatedEmulatorConnection {
    pub fn new(emulators: Vec<Box<dyn CTVEmulator>>, threshold: u8) -> Self {
        FederatedEmulatorConnection {
            emulators,
            threshold,
        }
    }
}

impl CTVEmulator for FederatedEmulatorConnection {
    fn get_signer_for(&self, h: Sha256) -> Result<Clause, EmulatorError> {
        let v = self
            .emulators
            .iter()
            .map(|e| e.get_signer_for(h))
            .collect::<Result<Vec<Clause>, EmulatorError>>()?;
        Ok(Clause::Threshold(self.threshold as usize, v))
    }
    fn sign(
        &self,
        mut b: PartiallySignedTransaction,
    ) -> Result<PartiallySignedTransaction, EmulatorError> {
        for emulator in self.emulators.iter() {
            b = emulator.sign(b)?;
        }
        Ok(b)
    }
}
