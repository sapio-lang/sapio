// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Hierarchical Deterministic Emulator Connection

use super::*;
use std::time::Duration;
/// HDOracleEmulatorConnection wraps a tokio runtime and a TCPStream
/// with a key to be able to talk to an Oracle server.
///
/// Note that because HDOracleEmulatorConnection uses block_in_place/block_on
/// internally in the trait object because the CTVEmulator trait is not async.
///
/// This seems to be a limitation with tokio / rust around using async inside non-async
/// traits.
pub struct HDOracleEmulatorConnection {
    /// the Oracle's runtime, if not executing in a runtime already
    pub runtime: Option<Arc<tokio::runtime::Runtime>>,
    /// handle to either current_runtime or the runtime owned above
    pub handle: tokio::runtime::Handle,
    /// connection to the reconnect SocketAddr
    pub connection: Mutex<Option<TcpStream>>,
    /// resolved address to the oracle
    pub reconnect: SocketAddr,
    /// the root key signatures will come from
    pub root: ExtendedPubKey,
    /// a secp context
    pub secp: Arc<bitcoin::secp256k1::Secp256k1<bitcoin::secp256k1::All>>,
    request_timeout: Duration,
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
        runtime: Option<Arc<tokio::runtime::Runtime>>,
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
            handle: Handle::try_current().unwrap_or_else(|_e| {
                runtime
                    .as_ref()
                    .expect("Must pass a runtime if not in async context")
                    .handle()
                    .clone()
            }),
            runtime,
            root,
            secp,
            request_timeout: crate::DEFAULT_REQUEST_TIMEOUT,
        })
    }

    /// Set the deadline for one signing request, including waiting for another
    /// request, connecting, and exchanging the complete response.
    ///
    /// The duration must be positive and representable by the runtime's clock.
    pub fn with_request_timeout(mut self, timeout: Duration) -> Result<Self, std::io::Error> {
        crate::validate_request_timeout(timeout)?;
        self.request_timeout = timeout;
        Ok(self)
    }

    async fn request(
        &self,
        request: PartiallySignedTransaction,
    ) -> Result<PartiallySignedTransaction, EmulatorError> {
        tokio::time::timeout(self.request_timeout, async {
            let mut mconn = self.connection.lock().await;
            // Only a fully validated exchange restores the cached connection.
            // Timeout or caller cancellation drops a partially consumed stream.
            let mut connection = match mconn.take() {
                Some(connection) => connection,
                None => TcpStream::connect(self.reconnect).await?,
            };
            crate::wire::write_message(
                &mut connection,
                &msgs::Request::SignPSBT(msgs::PSBT(request.clone())),
            )
            .await?;
            let response = crate::wire::read_message::<msgs::PSBT>(&mut connection).await?;
            sapio_ctv_emulator_trait::validate_signing_response(&request, &response.0)?;
            *mconn = Some(connection);
            Ok(response.0)
        })
        .await
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Emulator signing request deadline exceeded",
            )
        })?
    }
}

use tokio::{runtime::Handle, sync::Mutex};
impl CTVEmulator for HDOracleEmulatorConnection {
    fn get_signer_for(&self, h: Sha256) -> Result<Clause, EmulatorError> {
        Ok(Clause::Key(self.derive(h)?.to_x_only_pub()))
    }
    fn sign(
        &self,
        b: PartiallySignedTransaction,
    ) -> Result<PartiallySignedTransaction, EmulatorError> {
        tokio::task::block_in_place(|| self.handle.block_on(self.request(b)))
    }
}

#[cfg(test)]
mod tests;
