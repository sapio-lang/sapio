use super::*;
use crate::program::{EvaluatorId, MAX_PROGRAM_ROOT_DEPTH};
use bitcoin::bip32::Xpriv;
use bitcoin::secp256k1::Secp256k1;
use bitcoin::Network;

#[test]
fn public_fragment_constructors_preserve_exact_program_commitments() {
    let root = Xpub::from_priv(
        &Secp256k1::new(),
        &Xpriv::new_master(Network::Testnet, &[102; 32]).unwrap(),
    );
    let expected = sha256::Hash::hash(b"fixed template");
    let equality = template_hash_eq(expected, root).unwrap();
    assert_eq!(equality.instance().evaluator(), EvaluatorId::wasm_v2());
    assert_eq!(equality.instance().program(), TEMPLATEHASH_WASM);
    assert_eq!(equality.instance().parameters(), expected.as_byte_array());
    assert_eq!(*equality.root(), root);
    assert_eq!(
        equality.instance().id(),
        templatehash_wasm_instance(expected).id()
    );

    let key = root.public_key.x_only_public_key().0;
    for source in [
        TemplateKey::Pinned(key),
        TemplateKey::InternalKey,
        TemplateKey::KnownTweak,
    ] {
        let authorization = template_signed_by(source, root).unwrap();
        assert_eq!(
            authorization.instance(),
            &template_authorization_wasm_instance(source)
        );
        assert_eq!(*authorization.root(), root);
    }

    let mut deep_root = root;
    deep_root.depth = MAX_PROGRAM_ROOT_DEPTH + 1;
    for result in [
        template_hash_eq(expected, deep_root),
        template_signed_by(TemplateKey::InternalKey, deep_root),
    ] {
        assert_eq!(
            result,
            Err(ProgramError::RootDepth {
                depth: deep_root.depth
            })
        );
    }
}
