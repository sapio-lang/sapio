use lazy_static::lazy_static;
use sapio_contrib::contracts;
use sapio_front::session;
lazy_static! {
    pub static ref MENU : session::Menu = {
        let mut m = session::MenuBuilder::new();
        m.register_as::<contracts::ExampleA>("ExampleA".to_string().into());
        m.register_as::<contracts::ExampleB<contracts::Start>>("ExampleB".to_string().into());
        m.register_as::<contracts::treepay::TreePay>("TreePay".to_string().into());
        m.register_as::<contracts::hodl_chicken::HodlChickenInner>("HodlChicken".to_string().into());
        // Readme Contracts
        m.register_as::<contracts::readme_contracts::PayToPublicKey>("P2PK".to_string().into());
        m.register_as::<contracts::readme_contracts::BasicEscrow>("BasicEscrow".to_string().into());
        m.register_as::<contracts::readme_contracts::BasicEscrow2>("BasicEscrow2".to_string().into());
        m.register_as::<contracts::readme_contracts::TrustlessEscrow>("TrustlessEscrow".to_string().into());

        m.register_as_from::<contracts::vault::VaultAddress, contracts::vault::Vault, _>("Vault->Address".to_string().into());
        m.register_as_from::<contracts::vault::VaultTree, contracts::vault::Vault, _>("Vault->TreePay".to_string().into());

        m.register_as::<contracts::federated_sidechain::PegIn>("FederatedPegIn".to_string().into());

        m.into()
    };
}
