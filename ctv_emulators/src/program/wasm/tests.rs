use super::*;
use crate::program::{ProgramError, ProgramOracle, ProgramSigningRequest, ProgramSpendPath, PSBT};
use bitcoin::hashes::{hex::FromHex, Hash};
use bitcoin::secp256k1::Secp256k1;
use bitcoin::util::bip32::{ExtendedPrivKey, ExtendedPubKey};
use bitcoin::util::psbt::PartiallySignedTransaction;
use bitcoin::{Network, OutPoint, Script, Transaction, TxIn, TxOut};
use sapio_base::program::ProgramInstance;

fn module(body: &str, extra: &str, allocation: Option<&str>) -> Vec<u8> {
    let allocation = allocation
        .unwrap_or("global.get $heap global.get $heap local.get $length i32.add global.set $heap");
    wat::parse_str(format!(
        r#"(module
        (memory (export "memory") 1)
        (global $heap (mut i32) (i32.const 4096))
        (func (export "sapio_alloc_v1") (param $length i32) (result i32)
            {allocation})
        (func (export "sapio_evaluate_v1")
            (param $program i32) (param $program_len i32)
            (param $parameters i32) (param $parameters_len i32)
            (param $view i32) (param $view_len i32)
            (param $witness i32) (param $witness_len i32)
            (result i32) {body})
        {extra})"#
    ))
    .unwrap()
}

fn data() -> (Transaction, Vec<TxOut>) {
    (
        Transaction {
            version: -2,
            lock_time: 0x0102_0304,
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: bitcoin::Txid::from_inner([1; 32]),
                    vout: 0x1122_3344,
                },
                sequence: 0xaabb_ccdd,
                ..TxIn::default()
            }],
            output: vec![TxOut {
                value: 0xfedc_ba98_7654_3210,
                script_pubkey: Script::from(vec![0x51]),
            }],
        },
        vec![TxOut {
            value: 0x0102_0304_0506_0708,
            script_pubkey: Script::from(vec![0x00, 0x02, 0xab, 0xcd]),
        }],
    )
}

fn run(
    module: &[u8],
    program: &[u8],
    params: &[u8],
    witness: &[u8],
) -> Result<bool, EvaluationError> {
    let (transaction, outputs) = data();
    let prevouts: Vec<_> = outputs.iter().collect();
    let view = SignedTransactionView {
        transaction: &transaction,
        prevouts: &prevouts,
        input_index: 0,
    };
    evaluate(module, program, params, &view, witness)
}

fn root() -> ExtendedPrivKey {
    ExtendedPrivKey::new_master(Network::Testnet, &[42; 32]).unwrap()
}

fn request(instance: ProgramInstance) -> ProgramSigningRequest {
    let key = instance
        .derive_public_key(&ExtendedPubKey::from_priv(&Secp256k1::new(), &root()))
        .unwrap();
    let mut psbt = PartiallySignedTransaction::from_unsigned_tx(Transaction {
        version: 2,
        lock_time: 0,
        input: vec![TxIn::default()],
        output: vec![TxOut {
            value: 900,
            script_pubkey: Script::new(),
        }],
    })
    .unwrap();
    psbt.inputs[0].witness_utxo = Some(TxOut {
        value: 1_000,
        script_pubkey: Script::new_v1_p2tr(&Secp256k1::new(), key, None),
    });
    ProgramSigningRequest {
        instance,
        input_index: 0,
        witness: vec![9],
        path: ProgramSpendPath::KeyPath,
        psbt: PSBT(psbt),
    }
}

#[test]
fn zero_identity_runs_inline_wasm_and_registered_interpreters_receive_program_bytes() {
    let inline = module("local.get $program_len i32.eqz", "", None);
    let instance = ProgramInstance::new(EvaluatorId::wasm(), inline.clone(), vec![]).unwrap();
    let oracle = ProgramOracle::new(root(), vec![]).unwrap();
    let request = request(instance);
    let response = oracle.sign(request.clone()).unwrap();
    crate::program::validate_program_response(&request, &response, &oracle.public_root()).unwrap();

    let interpreter = WasmEvaluator::new(module(
        "local.get $program_len i32.const 1 i32.eq
         local.get $program i32.load8_u i32.const 7 i32.eq i32.and
         local.get $parameters i32.load8_u i32.const 8 i32.eq i32.and
         local.get $witness i32.load8_u i32.const 9 i32.eq i32.and",
        "",
        None,
    ))
    .unwrap();
    assert!(!interpreter.id().is_wasm());
    assert_eq!(
        interpreter.id(),
        EvaluatorId::for_wasm(interpreter.module())
    );
    let instance = ProgramInstance::new(interpreter.id(), vec![7], vec![8]).unwrap();
    let request = self::request(instance.clone());
    assert!(matches!(
        oracle.sign(request.clone()),
        Err(ProgramError::UnknownEvaluator(_))
    ));
    let registered = ProgramOracle::new(root(), vec![interpreter.clone()]).unwrap();
    registered.sign(request).unwrap();
    let altered = ProgramInstance::new(interpreter.id(), vec![6], vec![8]).unwrap();
    assert!(matches!(
        registered.sign(self::request(altered)),
        Err(ProgramError::Rejected)
    ));
    assert!(matches!(
        ProgramOracle::new(root(), vec![interpreter.clone(), interpreter]),
        Err(ProgramError::DuplicateEvaluator(_))
    ));
    // The public constructor computes IDs. Even an internal malformed value
    // cannot replace the reserved inline dispatcher during registration.
    let forged = WasmEvaluator {
        id: EvaluatorId::wasm(),
        module: inline.into(),
    };
    assert!(matches!(
        ProgramOracle::new(root(), vec![forged]),
        Err(ProgramError::ReservedEvaluator)
    ));
}

#[test]
fn only_boolean_results_accept_and_traps_never_produce_a_signature() {
    for (body, expected) in [
        ("i32.const 1", Some(true)),
        ("i32.const 0", Some(false)),
        ("i32.const 2", None),
        ("i32.const -1", None),
        ("unreachable", None),
    ] {
        let bytes = module(body, "", None);
        let result = run(&bytes, &[], &[], &[]);
        match expected {
            Some(value) => assert_eq!(result.unwrap(), value),
            None => assert!(result.is_err()),
        }
        let instance = ProgramInstance::new(EvaluatorId::wasm(), bytes, vec![]).unwrap();
        let result = ProgramOracle::new(root(), vec![])
            .unwrap()
            .sign(request(instance));
        if expected == Some(true) {
            assert!(result.is_ok());
        } else {
            assert!(result.is_err());
        }
    }
}

#[test]
fn every_request_has_fresh_globals_memory_and_allocation_state() {
    let bytes = module(
        "global.get $used i32.eqz
         i32.const 1 global.set $used",
        "(global $used (mut i32) (i32.const 0))",
        None,
    );
    for _ in 0..3 {
        assert!(run(&bytes, &[], &[1, 2, 3], &[4, 5]).unwrap());
    }
}

#[test]
fn all_allocations_finish_before_inputs_are_copied_and_aliases_are_rejected() {
    let aliases = module("i32.const 1", "", Some("i32.const 1024"));
    assert!(run(&aliases, &[1], &[2], &[3])
        .unwrap_err()
        .0
        .contains("overlap"));
    let partial_alias = module(
        "i32.const 1",
        "",
        Some("global.get $heap global.get $heap i32.const 1 i32.add global.set $heap"),
    );
    assert!(run(&partial_alias, &[1, 2], &[3, 4], &[])
        .unwrap_err()
        .0
        .contains("overlap"));
    for pointer in ["i32.const -1", "i32.const 65536", "i32.const 65535"] {
        let bytes = module("i32.const 1", "", Some(pointer));
        assert!(run(&bytes, &[], &[], &[])
            .unwrap_err()
            .0
            .contains("outside linear memory"));
    }
    let rewriting = module(
        "local.get $parameters i32.load8_u i32.const 7 i32.eq",
        "",
        Some(
            "i32.const 4096 i32.const 0 i32.store8
         global.get $heap global.get $heap local.get $length i32.add global.set $heap",
        ),
    );
    assert!(run(&rewriting, &[], &[7], &[1]).unwrap());
}

#[test]
fn allocation_and_evaluation_share_one_nonrenewable_fuel_budget() {
    let spinning = module("(loop br 0) unreachable", "", None);
    assert!(run(&spinning, &[], &[], &[])
        .unwrap_err()
        .0
        .contains("exhausted"));
    let spinning = module("i32.const 1", "", Some("(loop br 0) unreachable"));
    assert!(run(&spinning, &[], &[], &[])
        .unwrap_err()
        .0
        .contains("sapio_alloc_v1"));
    // Each call consumes 40M bulk-memory fuel without growing the instance.
    // Three allocations therefore exhaust the common 100M allowance even
    // though every individual allocation would fit in a fresh budget.
    let cumulative = wat::parse_str(
        r#"(module
        (memory (export "memory") 640)
        (global $heap (mut i32) (i32.const 4096))
        (func (export "sapio_alloc_v1") (param $length i32) (result i32)
            i32.const 0 i32.const 0 i32.const 40000000 memory.fill
            global.get $heap global.get $heap local.get $length i32.add global.set $heap)
        (func (export "sapio_evaluate_v1")
            (param i32 i32 i32 i32 i32 i32 i32 i32) (result i32) i32.const 1))"#,
    )
    .unwrap();
    assert!(run(&cumulative, &[], &[1], &[]).unwrap());
    assert!(run(&cumulative, &[], &[1], &[1])
        .unwrap_err()
        .0
        .contains("exhausted"));
}

#[test]
fn start_import_memory_and_export_capabilities_are_closed() {
    let start = module("i32.const 1", "(func $start) (start $start)", None);
    assert!(run(&start, &[], &[], &[])
        .unwrap_err()
        .0
        .contains("start function"));
    for imported in [
        r#"(import "wasi_snapshot_preview1" "random_get" (func (param i32 i32) (result i32)))"#,
        r#"(import "env" "sapio_v1_wasm_plugin_create_contract" (func))"#,
        r#"(import "sapio_crypto_v1" "unknown" (func))"#,
    ] {
        let source = format!(
            r#"(module {imported}
            (memory (export "memory") 1)
            (func (export "sapio_alloc_v1") (param i32) (result i32) i32.const 1024)
            (func (export "sapio_evaluate_v1") (param i32 i32 i32 i32 i32 i32 i32 i32) (result i32) i32.const 1))"#
        );
        assert!(run(&wat::parse_str(source).unwrap(), &[], &[], &[]).is_err());
    }
    for source in [
        r#"(module (memory (export "memory") 1025))"#,
        r#"(module (memory (export "memory") 1) (table 65537 funcref))"#,
        r#"(module (memory (export "other") 1))"#,
        r#"(module (memory (export "memory") 1) (func (export "sapio_alloc_v1") (result i32) i32.const 0))"#,
    ] {
        assert!(run(&wat::parse_str(source).unwrap(), &[], &[], &[]).is_err());
    }
    assert!(WasmEvaluator::new(vec![0; MAX_PROGRAM_BYTES + 1]).is_err());
    assert!(WasmEvaluator::new(b"(module)".to_vec()).is_err());
}

#[test]
fn compilation_budget_bounds_compact_locals_and_repeated_type_parameters() {
    fn leb(mut value: u32, bytes: &mut Vec<u8>) {
        loop {
            let byte = (value & 127) as u8;
            value >>= 7;
            bytes.push(byte | if value != 0 { 128 } else { 0 });
            if value == 0 {
                break;
            }
        }
    }
    fn local_module(counts: &[u32]) -> Vec<u8> {
        let mut bytes = b"\0asm\x01\0\0\0".to_vec();
        // One empty function type, then one defined function per local count.
        bytes.extend_from_slice(&[1, 4, 1, 0x60, 0, 0]);
        bytes.extend_from_slice(&[3, counts.len() as u8 + 1, counts.len() as u8]);
        bytes.extend(std::iter::repeat_n(0, counts.len()));
        let mut code = vec![counts.len() as u8];
        for count in counts {
            let mut body = vec![1];
            leb(*count, &mut body);
            body.extend_from_slice(&[0x7f, 0x0b]);
            leb(body.len() as u32, &mut code);
            code.extend(body);
        }
        bytes.push(10);
        leb(code.len() as u32, &mut bytes);
        bytes.extend(code);
        bytes
    }
    let compact = local_module(&[50_000, 50_000]);
    assert!(compact.len() < 64);
    assert!(run(&compact, &[], &[], &[])
        .unwrap_err()
        .0
        .contains("65536 function parameter/local slots"));
    for counts in [&[u32::MAX][..], &[1, u32::MAX][..]] {
        assert!(check_compilation_budget(&local_module(counts)).is_err());
    }
    assert!(check_compilation_budget(&local_module(&[32_768, 32_768])).is_ok());
    assert!(check_compilation_budget(&local_module(&[32_768, 32_769])).is_err());

    let repeated = wat::parse_str(format!(
        "(module (type $shared (func (param {}))) {})",
        "i32 ".repeat(512),
        "(func (type $shared)) ".repeat(129),
    ))
    .unwrap();
    assert!(repeated.len() < MAX_PROGRAM_BYTES);
    assert!(check_compilation_budget(&repeated)
        .unwrap_err()
        .0
        .contains("function parameter/local slots"));
    assert!(check_compilation_budget(&module("i32.const 1", "", None)).is_ok());

    // Repeated imports must not reach JIT signature expansion before their
    // capabilities and exact types have been checked.
    for name in ["unknown", "sha256"] {
        let imports = wat::parse_str(format!(
            "(module (type $large (func (param {}) (result i32))) {})",
            "i32 ".repeat(512),
            format!("(import \"sapio_crypto_v1\" \"{name}\" (func (type $large))) ").repeat(129),
        ))
        .unwrap();
        assert!(imports.len() < MAX_PROGRAM_BYTES);
        assert!(check_compilation_budget(&imports)
            .unwrap_err()
            .0
            .contains("import"));
    }
    let valid = wat::parse_str(
        r#"(module
        (import "sapio_crypto_v1" "sha256" (func (param i32 i32 i32) (result i32)))
        (import "sapio_crypto_v1" "bip32_derive" (func (param i32 i32 i32 i32) (result i32)))
        (import "sapio_crypto_v1" "schnorr_verify" (func (param i32 i32 i32) (result i32))))"#,
    )
    .unwrap();
    assert!(check_compilation_budget(&valid).is_ok());
}

#[test]
fn signed_view_encoding_has_fixed_field_order_widths_and_little_endian_amounts() {
    let (transaction, outputs) = data();
    let prevouts: Vec<_> = outputs.iter().collect();
    let view = SignedTransactionView {
        transaction: &transaction,
        prevouts: &prevouts,
        input_index: 0,
    };
    let expected = Vec::<u8>::from_hex(concat!(
        "feffffff040302010000000001000000",
        "0101010101010101010101010101010101010101010101010101010101010101",
        "44332211ddccbbaa0807060504030201040000000002abcd",
        "010000001032547698badcfe0100000051",
    ))
    .unwrap();
    assert_eq!(encode_view(&view).unwrap(), expected);
}

#[test]
fn signed_view_limit_is_checked_before_copying_large_scripts() {
    let (mut transaction, outputs) = data();
    let prevouts: Vec<_> = outputs.iter().collect();
    // The fixed encoding consumes 88 bytes before the output's script.
    transaction.output[0].script_pubkey = Script::from(vec![0; MAX_SIGNED_VIEW_BYTES - 88]);
    let view = SignedTransactionView {
        transaction: &transaction,
        prevouts: &prevouts,
        input_index: 0,
    };
    assert_eq!(encode_view(&view).unwrap().len(), MAX_SIGNED_VIEW_BYTES);
    transaction.output[0].script_pubkey = Script::from(vec![0; MAX_SIGNED_VIEW_BYTES - 87]);
    let view = SignedTransactionView {
        transaction: &transaction,
        prevouts: &prevouts,
        input_index: 0,
    };
    assert!(encode_view(&view).is_err());
}
