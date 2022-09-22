// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! James O'Beirne's Vault Demo

#![deny(missing_docs)]
use bitcoin::util::bip32::ExtendedPubKey;
use bitcoin::Amount;
use sapio::contract::actions::conditional_compile::ConditionalCompileType;
use sapio::contract::*;
use sapio::util::amountrange::{AmountF64, AmountU64};
use sapio::*;
use sapio_base::timelocks::AnyRelTimeLock;
use sapio_base::*;
use sapio_wasm_plugin::client::*;
use sapio_wasm_plugin::*;
use schemars::*;
use serde::*;
use std::marker::PhantomData;

/// A type tag which tracks state for compile_if inside of Vault
trait State: for<'a> Deserialize<'a> + JsonSchema + 'static {
    /// if is_redeeming state, require compilation
    fn is_redeeming() -> ConditionalCompileType {
        ConditionalCompileType::Required
    }
    /// if is_secure state, require compilation
    fn is_secure() -> ConditionalCompileType {
        ConditionalCompileType::Required
    }
}
/// # Secure
/// Type tag to indicate the vault is in secure mode
#[derive(JsonSchema, Deserialize)]
struct Secure;
/// # Redeeming
/// Type tag to indicate the vault is in redeeming mode
#[derive(JsonSchema, Deserialize)]
struct Redeeming;

impl State for Secure {
    fn is_redeeming() -> ConditionalCompileType {
        ConditionalCompileType::Never
    }
}
impl State for Redeeming {
    fn is_secure() -> ConditionalCompileType {
        ConditionalCompileType::Never
    }
}

/// # Output
/// Where to send funds.
#[derive(JsonSchema, Deserialize, Clone)]
struct Output {
    /// # Address
    /// The address to pay to
    address: bitcoin::Address,
    /// # Amount
    /// How much funds to pay in Bitcoin
    amount: AmountF64,
}

#[derive(JsonSchema, Deserialize)]
struct Vault<S: State> {
    /// # Timeout
    ///  How much time to wait before redeemable with hot_key after claim
    ///  initiated
    timeout: AnyRelTimeLock,
    /// # Hot Spending Key
    /// key which can be used to spend from the vault <timeout> after claim
    /// initiated.
    #[schemars(with = "String")]
    hot_key: ExtendedPubKey,
    /// # Backup Direct
    /// If available, this key can be used immediately as a single sig cold
    /// multisig option. usable with a musig key.
    #[schemars(with = "String")]
    backup: Option<ExtendedPubKey>,
    /// # Backup Address
    /// Where funds should land if they are backed up
    backup_addr: bitcoin::Address,
    /// # Default Fee
    /// How much feerate each transaction should have per virtual kilo-weight
    /// unit, in sats
    /// e.g., a  1 sat per virtual kilo-weight unit feerate would be 1000
    default_feerate: AmountU64,
    /// # CPFP Config
    /// If a CPFP anchor is to be added
    cpfp: Option<Output>,
    #[serde(skip, default)]
    pd: PhantomData<S>,
}

/// Helper to coerce a Contract's stateful arguments into itself.
fn default_coerce<T: Contract>(
    k: <T as Contract>::StatefulArguments,
) -> Result<<T as Contract>::StatefulArguments, CompilationError> {
    Ok(k)
}

impl<S: State> Vault<S> {
    /// don't compile spend_hot unless we're in redeeming mode
    #[compile_if]
    fn compile_spend_hot(self, _ctx: Context) {
        S::is_redeeming()
    }
    /// make a guard with the timeout condition and the hot key.
    #[guard]
    fn hot_key_cl(self, _ctx: Context) {
        Clause::And(vec![
            Clause::Key(self.hot_key.to_x_only_pub()),
            self.timeout.into(),
        ])
    }
    /// allow spending with the satisfaction of hot_key_cl, but only in state =
    /// Redeeming.
    #[continuation(
        guarded_by = "[Self::hot_key_cl]",
        compile_if = "[Self::compile_spend_hot]",
        coerce_args = "default_coerce::<Self>",
        web_api
    )]
    fn spend_hot(self, ctx: Context, u: Option<Output>) {
        if let Some(Output { address, amount }) = u {
            ctx.template()
                .set_label("spend via hot".into())
                .set_color("red".into())
                .set_sequence(-1, self.timeout.into())?
                .add_output(
                    amount.into(),
                    &Compiled::from_address(address, None),
                    Some(
                        [(
                            "purpose",
                            "Funds transfered out of vault to this address.".into(),
                        )]
                        .into(),
                    ),
                )?
                .into()
        } else {
            empty()
        }
    }
    /// a contract has_cold if a plain backup key has been provided. some cold
    /// storages won't have a plain key path
    #[compile_if]
    fn has_cold(self, _ctx: Context) {
        self.backup
            .and(Some(ConditionalCompileType::Required))
            .unwrap_or(ConditionalCompileType::Never)
    }
    /// cold_key is
    #[guard]
    fn cold_key(self, _ctx: Context) {
        return self
            .backup
            .clone()
            .map(|e| e.to_x_only_pub())
            .map(Clause::Key)
            .unwrap_or(Clause::Unsatisfiable);
    }
    /// allow direct spending with the cold key if there is one.
    /// skip this otherwise.
    #[continuation(
        guarded_by = "[Self::cold_key]",
        compile_if = "[Self::has_cold]",
        coerce_args = "default_coerce::<Self>",
        web_api
    )]
    fn spend_cold(self, ctx: Context, u: Option<Output>) {
        if let Some(Output { amount, address }) = u {
            ctx.template()
                .set_label("spend via cold direct".into())
                .set_color("cyan".into())
                .add_output(
                    amount.into(),
                    &Compiled::from_address(address, None),
                    Some(
                        [(
                            "purpose",
                            "Funds transfered out of vault to this address.".into(),
                        )]
                        .into(),
                    ),
                )?
                .into()
        } else {
            empty()
        }
    }
    /// send the funds to the backup address without delay.
    #[then]
    fn backup(self, ctx: Context) {
        let mut tmpl = ctx
            .template()
            .set_label("backup to cold".into())
            .set_color("darkblue".into());
        if let Some(Output { address, amount }) = self.cpfp.clone() {
            tmpl = tmpl.add_output(
                amount.into(),
                &Compiled::from_address(address, None),
                Some([("purpose", "CPFP Anchor Output".into())].into()),
            )?;
        }
        let size = tmpl.estimate_tx_size() + 8 + self.backup_addr.script_pubkey().len() as u64;
        tmpl = tmpl.spend_amount((Amount::from(self.default_feerate) * 4 * size) / 1000)?;
        let funds = tmpl.ctx().funds();
        tmpl = tmpl.add_output(
            funds,
            &Compiled::from_address(self.backup_addr.clone(), None),
            Some(
                [(
                    "purpose",
                    "Funds sent to higher security backup address.".into(),
                )]
                .into(),
            ),
        )?;
        tmpl.into()
    }
    /// Only allow redeeming to begin when we are in the Secure state.
    #[compile_if]
    fn compile_begin_redeem(self, _ctx: Context) {
        S::is_secure()
    }
    /// Move the funds from a vault state = Secure to a vault State = Redeeming
    #[then(compile_if = "[Self::compile_begin_redeem]")]
    fn begin_redeem(self, ctx: Context) {
        let mut tmpl = ctx
            .template()
            .set_label("begin redeem".into())
            .set_color("pink".into());
        if let Some(Output { address, amount }) = self.cpfp.clone() {
            tmpl = tmpl.add_output(
                amount.into(),
                &Compiled::from_address(address, None),
                Some([("purpose", "CPFP Anchor Output".into())].into()),
            )?;
        }
        let size = tmpl.estimate_tx_size() + 8 + 35 /* 1 byte len, 1 byte version, 1 byte len, 32 bytes data*/;
        tmpl = tmpl.spend_amount((Amount::from(self.default_feerate) * 4 * size) / 1000)?;
        let funds = tmpl.ctx().funds();
        tmpl = tmpl.add_output(
            funds,
            &Vault::<Redeeming> {
                backup: self.backup.clone(),
                hot_key: self.hot_key,
                backup_addr: self.backup_addr.clone(),
                cpfp: self.cpfp.clone(),
                default_feerate: self.default_feerate,
                timeout: self.timeout,
                pd: Default::default(),
            },
            Some(
                [(
                    "purpose",
                    "Funds being proposed to be withdrawn into hot wallet.".into(),
                )]
                .into(),
            ),
        )?;
        tmpl.into()
    }
}
impl<S: State + 'static> Contract for Vault<S> {
    declare! {updatable<Option<Output>>, Self::spend_cold, Self::spend_hot}
    declare! {then, Self::backup, Self::begin_redeem}
}
type JamesVault = Vault<Secure>;
REGISTER![JamesVault, "logo.png"];
