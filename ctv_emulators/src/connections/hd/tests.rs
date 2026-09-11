use super::*;
use bitcoin::{Network, ScriptBuf, Transaction, TxIn, TxOut};
use std::{future::Future, io::ErrorKind, pin::Pin, task::Poll};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const DEADLINE: Duration = Duration::from_secs(10);

fn request() -> Psbt {
    Psbt::from_unsigned_tx(Transaction {
        version: bitcoin::transaction::Version(2),
        lock_time: bitcoin::absolute::LockTime::from_consensus(0),
        input: vec![TxIn::default()],
        output: vec![TxOut {
            value: bitcoin::Amount::from_sat(1_000),
            script_pubkey: ScriptBuf::new(),
        }],
    })
    .unwrap()
}

async fn connected() -> (HDOracleEmulatorConnection, TcpListener, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let secp = Arc::new(Secp256k1::new());
    let root = Xpub::from_priv(
        &secp,
        &Xpriv::new_master(Network::Regtest, &[7; 32]).unwrap(),
    );
    let connection = HDOracleEmulatorConnection::new(address, root, None, secp)
        .await
        .unwrap()
        .with_request_timeout(DEADLINE)
        .unwrap();
    let (client, peer) = tokio::join!(TcpStream::connect(address), listener.accept());
    *connection.connection.lock().await = Some(client.unwrap());
    (connection, listener, peer.unwrap().0)
}

// Polling explicitly keeps the paused clock from advancing while establishing
// which part of the exchange is pending on the real local socket.
async fn assert_pending(mut future: Pin<&mut impl Future>) {
    std::future::poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

fn assert_timeout(result: Result<Psbt, EmulatorError>) {
    assert!(matches!(
        result,
        Err(EmulatorError::NetworkIssue(error)) if error.kind() == ErrorKind::TimedOut
    ));
}

async fn assert_closed(mut peer: TcpStream) {
    tokio::time::timeout(Duration::from_secs(2), async {
        let _: msgs::Request = crate::wire::read_message(&mut peer).await.unwrap();
        assert_eq!(peer.read(&mut [0]).await.unwrap(), 0);
    })
    .await
    .unwrap();
}

async fn assert_reconnect(connection: &HDOracleEmulatorConnection, listener: TcpListener) {
    tokio::time::timeout(Duration::from_secs(2), async {
        let peer = async {
            let (mut stream, _) = listener.accept().await.unwrap();
            for _ in 0..2 {
                let msgs::Request::SignPSBT(response) =
                    crate::wire::read_message(&mut stream).await.unwrap();
                crate::wire::write_message(&mut stream, &response)
                    .await
                    .unwrap();
            }
        };
        let requests = async {
            for _ in 0..2 {
                assert_eq!(connection.request(request()).await.unwrap(), request());
            }
        };
        tokio::join!(peer, requests);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn stalled_response_header_and_body_expire_and_reconnect() {
    for prefix in [vec![], vec![0, 0], vec![0, 0, 0, 100, b'{']] {
        let (connection, listener, mut peer) = connected().await;
        if !prefix.is_empty() {
            peer.write_all(&prefix).await.unwrap();
            connection
                .connection
                .lock()
                .await
                .as_ref()
                .unwrap()
                .readable()
                .await
                .unwrap();
        }
        tokio::time::pause();
        let mut exchange = Box::pin(connection.request(request()));
        assert_pending(exchange.as_mut()).await;
        tokio::time::advance(DEADLINE).await;
        assert_timeout(exchange.await);
        assert!(connection.connection.try_lock().unwrap().is_none());
        tokio::time::resume();
        assert_closed(peer).await;
        assert_reconnect(&connection, listener).await;
    }
}

#[tokio::test]
async fn queue_wait_consumes_the_same_deadline_as_the_exchange() {
    let (connection, _, peer) = connected().await;
    let queued_behind = connection.connection.lock().await;
    tokio::time::pause();
    let mut exchange = Box::pin(connection.request(request()));
    assert_pending(exchange.as_mut()).await;
    tokio::time::advance(Duration::from_secs(6)).await;
    assert_pending(exchange.as_mut()).await;
    drop(queued_behind);
    assert_pending(exchange.as_mut()).await;
    tokio::time::advance(Duration::from_secs(4)).await;
    assert_timeout(exchange.await);
    assert!(connection.connection.try_lock().unwrap().is_none());
    tokio::time::resume();
    assert_closed(peer).await;
}

#[tokio::test]
async fn queued_timeout_preserves_the_socket_owned_by_the_previous_request() {
    let (connection, _, _) = connected().await;
    let queued_behind = connection.connection.lock().await;
    tokio::time::pause();
    let mut exchange = Box::pin(connection.request(request()));
    assert_pending(exchange.as_mut()).await;
    tokio::time::advance(DEADLINE).await;
    assert_timeout(exchange.await);
    assert!(queued_behind.is_some());
}

#[tokio::test]
async fn response_trickle_does_not_restart_the_deadline() {
    let (connection, _, mut peer) = connected().await;
    peer.write_u32(100).await.unwrap();
    connection
        .connection
        .lock()
        .await
        .as_ref()
        .unwrap()
        .readable()
        .await
        .unwrap();
    tokio::time::pause();
    let mut exchange = Box::pin(connection.request(request()));
    assert_pending(exchange.as_mut()).await;
    for _ in 0..2 {
        tokio::time::advance(Duration::from_secs(4)).await;
        peer.write_all(b" ").await.unwrap();
        tokio::task::yield_now().await;
        assert_pending(exchange.as_mut()).await;
    }
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_timeout(exchange.await);
    assert!(connection.connection.try_lock().unwrap().is_none());
    tokio::time::resume();
    assert_closed(peer).await;
}

#[tokio::test]
async fn cancellation_discards_an_inflight_socket_and_allows_reconnect() {
    let (connection, listener, peer) = connected().await;
    let mut exchange = Box::pin(connection.request(request()));
    assert_pending(exchange.as_mut()).await;
    drop(exchange);
    assert!(connection.connection.try_lock().unwrap().is_none());
    assert_closed(peer).await;
    assert_reconnect(&connection, listener).await;
}

#[tokio::test]
async fn invalid_response_discards_the_socket_before_reconnecting() {
    let (connection, listener, mut peer) = connected().await;
    let mut changed = request();
    changed.unsigned_tx.output[0].value -= bitcoin::Amount::ONE_SAT;
    crate::wire::write_message(&mut peer, &msgs::PSBT(changed))
        .await
        .unwrap();
    assert!(matches!(
        connection.request(request()).await,
        Err(EmulatorError::InvalidResponse)
    ));
    assert!(connection.connection.try_lock().unwrap().is_none());
    assert_closed(peer).await;
    assert_reconnect(&connection, listener).await;
}

#[tokio::test]
async fn timeout_configuration_rejects_zero_and_unrepresentable_durations() {
    let (connection, _, _) = connected().await;
    assert!(matches!(
        connection.with_request_timeout(Duration::ZERO),
        Err(error) if error.kind() == ErrorKind::InvalidInput
    ));
    let (connection, _, _) = connected().await;
    assert!(matches!(
        connection.with_request_timeout(Duration::MAX),
        Err(error) if error.kind() == ErrorKind::InvalidInput
    ));
}
