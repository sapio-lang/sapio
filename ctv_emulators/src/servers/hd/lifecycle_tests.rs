use super::*;
use crate::{msgs, wire};
use bitcoin::{Network, Transaction, TxIn};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use tokio::time::{advance, timeout};

fn oracle() -> HDOracleEmulator {
    HDOracleEmulator::new(ExtendedPrivKey::new_master(Network::Regtest, &[17; 32]).unwrap())
}

fn request() -> PartiallySignedTransaction {
    let mut psbt = PartiallySignedTransaction::from_unsigned_tx(Transaction {
        version: 2,
        lock_time: 0,
        input: vec![TxIn::default()],
        output: vec![TxOut {
            value: 9_000,
            script_pubkey: Script::new(),
        }],
    })
    .unwrap();
    let secp = Secp256k1::new();
    let key = oracle()
        .derive(psbt.unsigned_tx.get_ctv_hash(0), &secp)
        .unwrap()
        .to_keypair(&secp)
        .x_only_public_key()
        .0;
    psbt.inputs[0].witness_utxo = Some(TxOut {
        value: 10_000,
        script_pubkey: Script::new_v1_p2tr(&secp, key, None),
    });
    psbt
}

async fn send_request(stream: &mut (impl AsyncRead + AsyncWrite + Unpin)) {
    wire::write_message(stream, &msgs::Request::SignPSBT(msgs::PSBT(request())))
        .await
        .unwrap();
}

async fn read_response(stream: &mut (impl AsyncRead + AsyncWrite + Unpin)) {
    let response = wire::read_message::<msgs::PSBT>(stream).await.unwrap().0;
    sapio_ctv_emulator_trait::validate_signing_response(&request(), &response).unwrap();
    assert!(response.inputs[0].tap_key_sig.is_some());
}

async fn start_server(max_connections: usize) -> (SocketAddr, JoinHandle<std::io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = oracle()
        .with_limits(Duration::from_secs(60), max_connections)
        .unwrap();
    (address, tokio::spawn(server.serve(listener)))
}

async fn stop_server(server: JoinHandle<std::io::Result<()>>) {
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

#[test]
fn rejects_limits_that_disable_progress() {
    for duration in [Duration::ZERO, Duration::MAX] {
        assert!(matches!(
            oracle().with_limits(duration, 1),
            Err(error) if error.kind() == ErrorKind::InvalidInput
        ));
    }
    assert!(matches!(
        oracle().with_limits(Duration::from_secs(1), 0),
        Err(error) if error.kind() == ErrorKind::InvalidInput
    ));
}

#[tokio::test(start_paused = true)]
async fn idle_partial_headers_and_partial_bodies_expire() {
    for partial_request in [vec![], vec![0], vec![0, 0, 0, 16, b'{']] {
        let (mut client, stream) = tokio::io::duplex(64);
        let server = oracle().with_limits(Duration::from_secs(5), 1).unwrap();
        let connection = tokio::spawn(async move { server.serve_connection(stream).await });
        client.write_all(&partial_request).await.unwrap();
        tokio::task::yield_now().await;
        advance(Duration::from_secs(5)).await;
        assert_eq!(
            connection.await.unwrap().unwrap_err().kind(),
            ErrorKind::TimedOut
        );
        assert_eq!(client.read(&mut [0]).await.unwrap(), 0);
    }
}

#[tokio::test(start_paused = true)]
async fn partial_progress_does_not_restart_the_deadline() {
    let (mut client, stream) = tokio::io::duplex(64);
    let server = oracle().with_limits(Duration::from_secs(5), 1).unwrap();
    let connection = tokio::spawn(async move { server.serve_connection(stream).await });
    client.write_u32(16).await.unwrap();
    tokio::task::yield_now().await;
    for _ in 0..4 {
        advance(Duration::from_secs(1)).await;
        client.write_u8(b' ').await.unwrap();
        tokio::task::yield_now().await;
        assert!(!connection.is_finished());
    }
    advance(Duration::from_secs(1)).await;
    assert_eq!(
        connection.await.unwrap().unwrap_err().kind(),
        ErrorKind::TimedOut
    );
}

#[tokio::test(start_paused = true)]
async fn a_peer_that_does_not_read_its_response_expires() {
    let (mut client, stream) = tokio::io::duplex(64);
    let server = oracle().with_limits(Duration::from_secs(5), 1).unwrap();
    let connection = tokio::spawn(async move { server.serve_connection(stream).await });
    send_request(&mut client).await;
    tokio::task::yield_now().await;
    advance(Duration::from_secs(5)).await;
    assert_eq!(
        connection.await.unwrap().unwrap_err().kind(),
        ErrorKind::TimedOut
    );
    let mut partial_response = vec![];
    client.read_to_end(&mut partial_response).await.unwrap();
    assert_eq!(partial_response.len(), 64);
}

#[tokio::test(start_paused = true)]
async fn a_completed_request_starts_a_fresh_deadline() {
    let (mut client, stream) = tokio::io::duplex(4_096);
    let server = oracle().with_limits(Duration::from_secs(5), 1).unwrap();
    let connection = tokio::spawn(async move { server.serve_connection(stream).await });
    tokio::task::yield_now().await;
    for _ in 0..2 {
        advance(Duration::from_secs(4)).await;
        send_request(&mut client).await;
        read_response(&mut client).await;
        assert!(!connection.is_finished());
    }
    advance(Duration::from_secs(5)).await;
    assert_eq!(
        connection.await.unwrap().unwrap_err().kind(),
        ErrorKind::TimedOut
    );
}

#[tokio::test]
async fn peer_errors_do_not_stop_the_listener() {
    let (address, server) = start_server(1).await;
    for invalid_frame in [vec![], vec![0, 0, 0, 0], vec![0, 0, 0, 1, b'!']] {
        let mut bad = TcpStream::connect(address).await.unwrap();
        bad.write_all(&invalid_frame).await.unwrap();
        drop(bad);
        let mut healthy = TcpStream::connect(address).await.unwrap();
        send_request(&mut healthy).await;
        timeout(Duration::from_secs(2), read_response(&mut healthy))
            .await
            .unwrap();
        assert!(!server.is_finished());
    }
    stop_server(server).await;
}

#[tokio::test]
async fn capacity_is_reused_after_a_connection_closes() {
    let (address, server) = start_server(1).await;
    let mut first = TcpStream::connect(address).await.unwrap();
    send_request(&mut first).await;
    read_response(&mut first).await;

    let mut waiting = TcpStream::connect(address).await.unwrap();
    send_request(&mut waiting).await;
    assert!(timeout(Duration::from_millis(50), waiting.peek(&mut [0]))
        .await
        .is_err());

    drop(first);
    timeout(Duration::from_secs(2), read_response(&mut waiting))
        .await
        .unwrap();
    stop_server(server).await;
}

#[tokio::test]
async fn cancelling_the_listener_closes_active_connections() {
    let (address, server) = start_server(2).await;
    let mut first = TcpStream::connect(address).await.unwrap();
    let mut second = TcpStream::connect(address).await.unwrap();
    for client in [&mut first, &mut second] {
        send_request(client).await;
        read_response(client).await;
    }
    stop_server(server).await;
    for client in [&mut first, &mut second] {
        assert_eq!(
            timeout(Duration::from_secs(2), client.read(&mut [0]))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }
}
