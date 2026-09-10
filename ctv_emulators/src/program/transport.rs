//! Explicit program-signing protocol over bounded length-prefixed JSON.

use super::{validate_program_response, ProgramError, ProgramOracle, ProgramSigningRequest, PSBT};
use bitcoin::util::{bip32::ExtendedPubKey, psbt::PartiallySignedTransaction};
use sapio_base::program::MAX_PROGRAM_ROOT_DEPTH;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

// The explicit version fixes both instance commitments and the signed view.
// A CTV-only server cannot decode this as a request to sign a template hash.
#[derive(Serialize, Deserialize)]
enum Request<T> {
    SignProgramV1(T),
}

#[derive(Serialize, Deserialize)]
enum Response {
    SignedV1(PSBT),
    RejectedV1(String),
}

/// A peer exchange failed or did not provide the requested program signature.
#[derive(Debug)]
pub enum ProgramClientError {
    /// Connection, framing or elapsed I/O deadline failure.
    Transport(io::Error),
    /// The remote evaluator refused the request; its text is diagnostic only.
    Rejected(String),
    /// The response failed local signature or PSBT integrity checks.
    Validation(ProgramError),
}

impl fmt::Display for ProgramClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => write!(f, "program signing transport: {error}"),
            Self::Rejected(reason) => write!(f, "program signing rejected: {reason}"),
            Self::Validation(error) => write!(f, "invalid program signing response: {error}"),
        }
    }
}

impl std::error::Error for ProgramClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            Self::Validation(error) => Some(error),
            Self::Rejected(_) => None,
        }
    }
}

impl From<io::Error> for ProgramClientError {
    fn from(error: io::Error) -> Self {
        Self::Transport(error)
    }
}

/// An explicit endpoint and public program-signing root.
///
/// Each request uses a fresh connection. There is no shared socket state,
/// automatic retry, CTV fallback or compiler callback. The elapsed deadline
/// covers connecting and the complete request/response exchange. Synchronous
/// serialization and cryptographic verification are not preemptible by it.
#[derive(Clone, Debug)]
pub struct ProgramClient {
    address: SocketAddr,
    root: ExtendedPubKey,
    request_timeout: Duration,
}

impl ProgramClient {
    /// Configure an already resolved endpoint with the default 30-second limit.
    pub fn new(address: SocketAddr, root: ExtendedPubKey) -> io::Result<Self> {
        if root.depth > MAX_PROGRAM_ROOT_DEPTH {
            return Err(crate::input_err(
                "Program signer root is too deep for derivation",
            ));
        }
        Ok(Self {
            address,
            root,
            request_timeout: crate::DEFAULT_REQUEST_TIMEOUT,
        })
    }

    /// Change the request deadline; zero and unrepresentable durations fail.
    pub fn with_request_timeout(mut self, timeout: Duration) -> io::Result<Self> {
        crate::validate_request_timeout(timeout)?;
        self.request_timeout = timeout;
        Ok(self)
    }

    /// The public root whose derived signature every response must contain.
    pub fn root(&self) -> &ExtendedPubKey {
        &self.root
    }

    /// Evaluate and sign exactly one program, input and spending path.
    ///
    /// Success requires a valid signature from the configured derived key.
    /// Other signatures and every non-signature PSBT field must be preserved.
    pub async fn sign(
        &self,
        request: ProgramSigningRequest,
    ) -> Result<PartiallySignedTransaction, ProgramClientError> {
        tokio::time::timeout(self.request_timeout, async {
            let mut stream = TcpStream::connect(self.address).await?;
            crate::wire::write_message(&mut stream, &Request::SignProgramV1(&request)).await?;
            let response: Response = crate::wire::read_message(&mut stream).await?;
            match response {
                Response::RejectedV1(reason) => Err(ProgramClientError::Rejected(reason)),
                Response::SignedV1(PSBT(psbt)) => {
                    validate_program_response(&request, &psbt, &self.root)
                        .map_err(ProgramClientError::Validation)?;
                    Ok(psbt)
                }
            }
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Program signing request timed out"))?
    }
}

impl ProgramOracle {
    /// Serve a prebound listener with 64 admitted connections and 30-second I/O.
    pub async fn serve(self, listener: TcpListener) -> io::Result<()> {
        self.serve_with_limits(listener, crate::DEFAULT_REQUEST_TIMEOUT, 64)
            .await
    }

    /// Serve with explicit admission and per-request I/O limits.
    ///
    /// At capacity, connections wait in the operating system's backlog. Each
    /// admitted request has one deadline across header, body and response.
    /// Cancelling this future aborts its active connections. Peer failures do
    /// not stop the listener; unexpected task failures do.
    ///
    /// WASM execution and crypto imports have their own shared fuel allowance.
    /// This I/O deadline cannot preempt synchronous module validation or
    /// compilation; evaluator module bytes are bounded before either step.
    pub async fn serve_with_limits(
        self,
        listener: TcpListener,
        request_timeout: Duration,
        max_connections: usize,
    ) -> io::Result<()> {
        crate::validate_request_timeout(request_timeout)?;
        if max_connections == 0 {
            return Err(crate::input_err(
                "Program oracle connection limit must be nonzero",
            ));
        }
        let oracle = Arc::new(self);
        let mut connections = JoinSet::new();
        loop {
            tokio::select! {
                completed = connections.join_next(), if !connections.is_empty() => {
                    let _peer_result = completed.expect("a connection task is present")
                        .map_err(io::Error::other)?;
                }
                accepted = listener.accept(), if connections.len() < max_connections => {
                    let (stream, _) = accepted?;
                    let oracle = oracle.clone();
                    connections.spawn(async move {
                        serve_connection(&oracle, stream, request_timeout).await
                    });
                }
            }
        }
    }
}

async fn serve_connection(
    oracle: &ProgramOracle,
    mut stream: impl AsyncRead + AsyncWrite + Unpin,
    timeout: Duration,
) -> io::Result<()> {
    loop {
        tokio::time::timeout(timeout, async {
            let Request::SignProgramV1(request): Request<ProgramSigningRequest> =
                crate::wire::read_message(&mut stream).await?;
            let response = match oracle.sign(request) {
                Ok(psbt) => Response::SignedV1(PSBT(psbt)),
                Err(error) => Response::RejectedV1(error.to_string()),
            };
            crate::wire::write_message(&mut stream, &response).await
        })
        .await
        .map_err(|_| {
            io::Error::new(io::ErrorKind::TimedOut, "Program oracle request timed out")
        })??;
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
