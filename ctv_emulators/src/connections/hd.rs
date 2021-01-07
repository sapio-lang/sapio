use super::*;
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

    async fn request(t: &mut TcpStream, r: &msgs::Request) -> Result<(), std::io::Error> {
        let v = serde_json::to_vec(r)?;
        t.write_u32(v.len() as u32).await?;
        t.write_all(&v[..]).await
    }
    async fn response<T: DeserializeOwned + Clone>(t: &mut TcpStream) -> Result<T, std::io::Error> {
        let l = t.read_u32().await? as usize;
        let mut v = vec![0u8; l];
        t.read_exact(&mut v[..]).await?;
        let t: T = serde_json::from_slice::<T>(&v[..])?;
        Ok(t)
    }
}
use core::future::Future;
use tokio::sync::Mutex;
impl CTVEmulator for HDOracleEmulatorConnection {
    fn get_signer_for(&self, h: Sha256) -> Result<Clause, EmulatorError> {
        Ok(Clause::Key(self.derive(h)?.public_key))
    }
    fn sign(
        &self,
        mut b: PartiallySignedTransaction,
    ) -> Result<PartiallySignedTransaction, EmulatorError> {
        let inp: Result<PartiallySignedTransaction, std::io::Error> =
            self.runtime.block_on(async {
                let mut mconn = self.connection.lock().await;
                loop {
                    if let Some(conn) = &mut *mconn {
                        Self::request(conn, &msgs::Request::SignPSBT(msgs::PSBT(b.clone())))
                            .await?;
                        conn.flush().await?;
                        return Ok(Self::response::<msgs::PSBT>(conn).await?.0);
                    } else {
                        *mconn = Some(TcpStream::connect(&self.reconnect).await?);
                    }
                }
            });

        b.merge(inp?)
            .or_else(|_e| input_error("Fault Signed PSBT"))?;
        Ok(b)
    }
}
