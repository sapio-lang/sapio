use super::*;
use crate::program::{ProgramSpendPath, WasmEvaluator};
use bitcoin::bip32::Xpriv;
use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::Secp256k1;
use bitcoin::{Network, ScriptBuf, Transaction, TxIn, TxOut};
use sapio_base::program::{EvaluatorId, ProgramInstance};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn evaluator() -> WasmEvaluator {
    WasmEvaluator::new(
        wat::parse_str(
            r#"(module
        (memory (export "memory") 1)
        (global $heap (mut i32) (i32.const 1024))
        (func (export "sapio_alloc_v1") (param $length i32) (result i32)
            global.get $heap
            global.get $heap local.get $length i32.add global.set $heap)
        (func (export "sapio_evaluate_v1")
            (param i32 i32 i32 i32 i32 i32 i32 i32) (result i32) i32.const 1))"#,
        )
        .unwrap(),
    )
    .unwrap()
}

fn root() -> Xpriv {
    Xpriv::new_master(Network::Testnet, &[73; 32]).unwrap()
}

fn public_root() -> Xpub {
    Xpub::from_priv(&Secp256k1::new(), &root())
}

fn oracle() -> ProgramOracle {
    ProgramOracle::new(root(), vec![evaluator()]).unwrap()
}

fn request() -> ProgramSigningRequest {
    let instance = ProgramInstance::new(evaluator().id(), vec![1], vec![]).unwrap();
    let key = instance.derive_public_key(&public_root()).unwrap();
    let mut psbt = Psbt::from_unsigned_tx(Transaction {
        version: bitcoin::transaction::Version(2),
        lock_time: bitcoin::absolute::LockTime::from_consensus(0),
        input: vec![TxIn::default()],
        output: vec![TxOut {
            value: bitcoin::Amount::from_sat(900),
            script_pubkey: ScriptBuf::new(),
        }],
    })
    .unwrap();
    psbt.inputs[0].witness_utxo = Some(TxOut {
        value: bitcoin::Amount::from_sat(1_000),
        script_pubkey: ScriptBuf::new_p2tr(&Secp256k1::new(), key, None),
    });
    psbt.inputs[0].tap_internal_key = Some(key);
    ProgramSigningRequest {
        instance,
        input_index: 0,
        witness: vec![],
        path: ProgramSpendPath::KeyPath,
        psbt: PSBT(psbt),
    }
}

async fn write_request(stream: &mut TcpStream) {
    crate::wire::write_message(stream, &Request::SignProgramV1(request()))
        .await
        .unwrap();
}

async fn read_signature(stream: &mut TcpStream) {
    let Response::SignedV1(PSBT(response)) = crate::wire::read_message(stream).await.unwrap()
    else {
        panic!("oracle rejected a valid request");
    };
    validate_program_response(&request(), &response, &public_root()).unwrap();
}

#[tokio::test]
async fn client_checks_success_and_rejection_without_a_protocol_fallback() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(oracle().serve(listener));
    let client = ProgramClient::new(address, public_root()).unwrap();
    let response = client.sign(request()).await.unwrap();
    assert!(response.inputs[0].tap_key_sig.is_some());

    let mut unsupported = request();
    unsupported.instance = ProgramInstance::new(
        EvaluatorId(sha256::Hash::hash(b"unregistered evaluator")),
        vec![],
        vec![],
    )
    .unwrap();
    assert!(matches!(
        client.sign(unsupported).await,
        Err(ProgramClientError::Rejected(_))
    ));
    // A rejected request closes only its own client connection.
    client.sign(request()).await.unwrap();

    let mut legacy = TcpStream::connect(address).await.unwrap();
    crate::wire::write_message(&mut legacy, &crate::msgs::Request::SignPSBT(request().psbt))
        .await
        .unwrap();
    assert_eq!(legacy.read(&mut [0]).await.unwrap(), 0);
    client.sign(request()).await.unwrap();
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn a_success_label_without_the_required_signature_is_rejected() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let Request::SignProgramV1(request): Request<ProgramSigningRequest> =
            crate::wire::read_message(&mut socket).await.unwrap();
        crate::wire::write_message(&mut socket, &Response::SignedV1(request.psbt))
            .await
            .unwrap();
    });
    let client = ProgramClient::new(address, public_root()).unwrap();
    assert!(matches!(
        client.sign(request()).await,
        Err(ProgramClientError::Validation(_))
    ));
    peer.await.unwrap();
}

#[tokio::test]
async fn response_deadline_discards_the_connection_and_allows_a_later_request() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let _: Request<ProgramSigningRequest> =
            crate::wire::read_message(&mut socket).await.unwrap();
        socket.write_u32(16).await.unwrap();
        socket.write_all(b"{").await.unwrap();
        // The client must close its incomplete response connection at expiry.
        assert_eq!(socket.read(&mut [0]).await.unwrap(), 0);
        let (mut next, _) = listener.accept().await.unwrap();
        let Request::SignProgramV1(request): Request<ProgramSigningRequest> =
            crate::wire::read_message(&mut next).await.unwrap();
        let signed = oracle().sign(request).unwrap();
        crate::wire::write_message(&mut next, &Response::SignedV1(PSBT(signed)))
            .await
            .unwrap();
    });
    let client = ProgramClient::new(address, public_root())
        .unwrap()
        .with_request_timeout(Duration::from_millis(100))
        .unwrap();
    assert!(matches!(
        client.sign(request()).await,
        Err(ProgramClientError::Transport(error)) if error.kind() == io::ErrorKind::TimedOut
    ));
    client
        .with_request_timeout(Duration::from_secs(5))
        .unwrap()
        .sign(request())
        .await
        .unwrap();
    peer.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn incomplete_request_bytes_do_not_extend_the_server_deadline() {
    let (mut client, server) = tokio::io::duplex(64);
    let task =
        tokio::spawn(
            async move { serve_connection(&oracle(), server, Duration::from_secs(5)).await },
        );
    client.write_u32(16).await.unwrap();
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(4)).await;
    client.write_all(b"{").await.unwrap();
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(
        task.await.unwrap().unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    assert_eq!(client.read(&mut [0]).await.unwrap(), 0);
}

#[tokio::test]
async fn admitted_connections_are_bounded_reused_and_owned_by_the_listener() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(oracle().serve_with_limits(listener, Duration::from_secs(30), 1));
    let mut first = TcpStream::connect(address).await.unwrap();
    write_request(&mut first).await;
    read_signature(&mut first).await;
    let mut second = TcpStream::connect(address).await.unwrap();
    write_request(&mut second).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(30), second.read_u8())
            .await
            .is_err()
    );
    drop(first);
    tokio::time::timeout(Duration::from_secs(5), read_signature(&mut second))
        .await
        .unwrap();
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
    assert_eq!(second.read(&mut [0]).await.unwrap(), 0);
}

#[tokio::test]
async fn invalid_configuration_is_rejected_before_serving() {
    let address = "127.0.0.1:0".parse().unwrap();
    let mut too_deep = public_root();
    too_deep.depth = MAX_PROGRAM_ROOT_DEPTH + 1;
    assert!(ProgramClient::new(address, too_deep).is_err());
    for timeout in [Duration::ZERO, Duration::MAX] {
        assert!(ProgramClient::new(address, public_root())
            .unwrap()
            .with_request_timeout(timeout)
            .is_err());
        let listener = TcpListener::bind(address).await.unwrap();
        assert!(oracle()
            .serve_with_limits(listener, timeout, 1)
            .await
            .is_err());
    }
    let listener = TcpListener::bind(address).await.unwrap();
    assert!(oracle()
        .serve_with_limits(listener, Duration::from_secs(1), 0)
        .await
        .is_err());
}
