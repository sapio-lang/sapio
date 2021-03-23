// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;
/// HDOracleEmulatorConnection wraps a tokio runtime and a TCPStream
/// with a key to be able to talk to an Oracle server.
///
/// Note that because HDOracleEmulatorConnection uses block_in_place/block_on
/// internally in the trait object because the CTVEmulator trait is not async.
///
/// This seems to be a limitation with tokio / rust around using async inside non-async
/// traits.
pub struct HDOracleEmulatorConnection {
    pub runtime: Arc<tokio::runtime::Runtime>,
    pub connection: Mutex<Option<TcpStream>>,
    pub reconnect: SocketAddr,
    pub root: ExtendedPubKey,
    pub secp: Arc<bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::All>>,
}

impl HDOracleEmulatorConnection {
    /// Helper function to derive an EPK
    fn derive(&self, h: Sha256) -> Result<ExtendedPubKey, Error> {
        let c = hash_to_child_vec(h);
        self.root.derive_pub(&self.secp, &c)
    }
    /// Creates a new instance of a HDOracleEmulatorConnection.
    ///
    /// Note that the runtime and secp can be shared with other instances as it is Arc.
    ///
    /// `new` does not connect to the address passed in immediately, but it does
    /// use tokio::net::lookup_host to resolve the address. A connection is not
    /// opened to the server until a call to the `sign` method is made. This is
    /// purposeful so that connections are not opened until they are actually needed.
    ///
    /// Note that as a consequence of new doing the host resolving, if DNS
    /// records change, then a new HDOracleEmulatorConnection would need to be
    /// created to observe it.
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

    /// make a request via the tcpstream.
    /// wire format: length:u32 data:[u8;length]
    async fn request(t: &mut TcpStream, r: &msgs::Request) -> Result<(), std::io::Error> {
        let v = serde_json::to_vec(r)?;
        t.write_u32(v.len() as u32).await?;
        t.write_all(&v[..]).await
    }
    /// receive a response via the tcpstream.
    /// wire format: length:u32 data:[u8;length]
    ///
    /// TODO: secure response by limiting the length to a max value.
    /// This is not super critical because presumably the oracles are not trying to OOM your system.
    async fn response<T: DeserializeOwned + Clone>(t: &mut TcpStream) -> Result<T, std::io::Error> {
        let l = t.read_u32().await? as usize;
        let mut v = vec![0u8; l];
        t.read_exact(&mut v[..]).await?;
        let t: T = serde_json::from_slice::<T>(&v[..])?;
        Ok(t)
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
        let inp: Result<PartiallySignedTransaction, std::io::Error> =
            tokio::task::block_in_place(|| {
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
                })
            });

        b.merge(inp?)
            .or_else(|_e| input_error("Fault Signed PSBT"))?;
        Ok(b)
    }
}
