use bitcoin::hashes::hex::ToHex;
use bitcoin::hashes::sha256::Hash as Sha256;
use bitcoin::hashes::Hash;
use bitcoin::util::amount::Amount;
use bitcoin::XOnlyPublicKey;
use sapio::contract::empty;
use sapio::contract::object::ObjectMetadata;
use sapio::contract::CompilationError;
use sapio::contract::Contract;
use sapio::*;
use sapio_base::Clause;
use sapio_wasm_nft_trait::*;
use sapio_wasm_plugin::client::*;
use sapio_wasm_plugin::plugin_handle::PluginHandle;
use sapio_wasm_plugin::*;
use schemars::*;
use serde::*;
use std::convert::TryFrom;
use std::convert::TryInto;
use std::sync::Arc;
/// # SimpleNFT
/// A really simple NFT... not much too it!
#[derive(JsonSchema, Serialize, Deserialize)]
pub struct SimpleNFT {
    /// The minting data, and nothing else.
    data: Mint_NFT_Trait_Version_0_1_0,
}

/// # The SimpleNFT Contract
impl Contract for SimpleNFT {
    // NFTs... only good for selling?
    declare! {updatable<Sell>, Self::sell}
    // embeds metadata
    declare! {finish, Self::metadata_commit}
    fn metadata(&self, _ctx: Context) -> Result<ObjectMetadata, CompilationError> {
        Ok(ObjectMetadata::default().add_simp(self.data.ipfs_nft.clone())?)
    }
    fn ensure_amount(&self, ctx: Context) -> Result<Amount, CompilationError> {
        Ok(ctx.funds())
    }
}

impl SimpleNFT {
    /// # unspendable
    /// what? This is just a sneaky way of making a provably unspendable branch
    /// (since the preimage of [0u8; 32] hash can never be found). We use that to
    /// help us embed metadata inside of our contract...
    /// TODO: Check this is OK
    #[guard]
    fn metadata_commit(self, _ctx: Context) {
        Clause::And(vec![
            Clause::Key(
                XOnlyPublicKey::from_slice(&Sha256::hash(&[1u8; 32]).into_inner())
                    .expect("constant"),
            ),
            Clause::Sha256(self.data.ipfs_nft.commitment()),
        ])
    }
    /// # signed
    /// Get the current owners signature.
    #[guard]
    fn signed(self, _ctx: Context) {
        Clause::Key(self.data.owner.clone())
    }
}
fn default_coerce(k: <SimpleNFT as Contract>::StatefulArguments) -> Result<Sell, CompilationError> {
    Ok(k)
}

impl SellableNFT for SimpleNFT {
    #[continuation(guarded_by = "[Self::signed]", web_api, coerce_args = "default_coerce")]
    fn sell(self, mut ctx: Context, sale: Sell) {
        if let Sell::MakeSale {
            sale_info_partial,
            mut which_sale,
        } = sale
        {
            let sale_info = sale_info_partial.fill(self.data.clone());
            let sale_ctx = ctx.derive_str(Arc::new("sell".into()))?;
            // create a contract from the sale API passed in
            let create_args: CreateArgs<sale_impl::Versions> = CreateArgs {
                context: ContextualArguments {
                    amount: ctx.funds(),
                    network: ctx.network,
                    effects: unsafe { ctx.get_effects_internal() }.as_ref().clone(),
                    ordinals_info: ctx.get_ordinals().clone(),
                },
                arguments: sale_impl::Versions::NFT_Sale_Trait_Version_0_1_0(sale_info.clone()),
            };
            // use the sale API we passed in
            let compiled = which_sale.call(sale_ctx.path(), &create_args)?;
            // send to this sale!
            let pays = compiled.amount_range.max() - ctx.funds();
            let mut builder = ctx.template().add_amount(pays);
            // todo: we need to cut-through the compiled contract address, but this
            // upgrade to Sapio semantics will come Soon™.
            builder = builder.add_output(compiled.amount_range.max(), &compiled, None)?;
            // for now, a capital H Hack.
            builder = builder.add_sequence();
            builder.into()
        } else {
            // Don't do anything if we're holding!
            empty()
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema)]
enum Versions {
    Mint_NFT_Trait_Version_0_1_0(Mint_NFT_Trait_Version_0_1_0),
}

impl TryFrom<Versions> for SimpleNFT {
    type Error = CompilationError;
    fn try_from(v: Versions) -> Result<Self, Self::Error> {
        let Versions::Mint_NFT_Trait_Version_0_1_0(mut data) = v;
        let this: NFTMintingModule = LookupFrom::This
            .try_into()
            .map_err(|_| CompilationError::TerminateWith("Failed to Lookup".into()))?;
        // required otherwise cross-moudle calls get bungled
        // TODO: Address this more suavely
        let this = this.canonicalize();
        match data.minting_module {
            // if no module is provided, it must be this module!
            None => {
                data.minting_module = Some(this);
                Ok(SimpleNFT { data })
            }
            // if a module is provided, we have no idea what to do...
            // unless the module is this module itself!
            Some(ref mut module) if module.key == this.key => {
                *module = module.canonicalize();
                Ok(SimpleNFT { data })
            }
            _ => Err(CompilationError::TerminateWith(format!(
                "Minting module must be None or equal to {}",
                this.key.to_hex()
            ))),
        }
    }
}
REGISTER![[SimpleNFT, Versions], "logo.png"];
