//! Typed command-line inputs. Runtime configuration is loaded only by its users.

use bitcoin::{Network, OutPoint};
use clap::{Args, Parser, Subcommand};
use emulator_connect::program::SpendPath;
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "sapio-cli",
    version,
    about = "Compile, inspect and spend Sapio contracts"
)]
pub(crate) struct Cli {
    /// Runtime configuration for binding, emulator and configuration commands.
    /// Local module, explain and spend commands do not read this file.
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Inspect or create runtime configuration.
    Configure {
        #[command(subcommand)]
        command: Configure,
    },
    /// Create keys and sign PSBTs locally.
    Signer {
        #[command(subcommand)]
        command: Signer,
    },
    /// Sapio Studio's JSON request/response protocol.
    Studio {
        #[command(subcommand)]
        command: Studio,
    },
    /// Run or contact explicitly configured CTV emulator servers.
    Emulator {
        #[command(subcommand)]
        command: Emulator,
    },
    /// Perform operations on PSBTs.
    Psbt {
        #[command(subcommand)]
        command: Psbt,
    },
    /// Compile, inspect, bind and complete contracts.
    Contract {
        #[command(subcommand)]
        command: Contract,
    },
}

#[derive(Subcommand)]
pub(crate) enum Configure {
    /// Show the configuration file and the actual default module cache.
    Files {
        #[arg(short, long)]
        json: bool,
    },
    /// Show the active configuration with passwords redacted.
    Show,
    /// Create a configuration using Bitcoin cookie-file authentication.
    Wizard {
        /// Save a new configuration at --config or the default location.
        #[arg(short, long)]
        write: bool,
    },
}

#[derive(Args)]
pub(crate) struct Output {
    /// Write a new file; omit or use - for stdout. Existing files are never replaced.
    #[arg(short, long)]
    pub output: Option<PathBuf>,
}

#[derive(Subcommand)]
pub(crate) enum Signer {
    /// Sign a base64 PSBT with an explicitly selected binary Xpriv key file.
    Sign {
        #[arg(short, long)]
        key: PathBuf,
        /// Base64 PSBT file; omit or use - for stdin.
        #[arg(short, long)]
        psbt: Option<PathBuf>,
        #[command(flatten)]
        output: Output,
    },
    /// Create a private key file and print its extended public key.
    New {
        #[arg(short, long)]
        network: Network,
        /// New private key file. Existing files are never replaced.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Show the extended public key of a binary Xpriv key file.
    Show {
        #[arg(short, long)]
        input: PathBuf,
    },
}

#[derive(Subcommand)]
pub(crate) enum Studio {
    /// Serve successive JSON requests on stdin and responses on stdout.
    Server {
        #[arg(long, required = true)]
        stdin: bool,
    },
    /// Print JSON schemas for the Studio protocol.
    Schemas,
}

#[derive(Subcommand)]
pub(crate) enum Emulator {
    /// Ask the configured CTV emulator to sign a base64 PSBT.
    Sign {
        /// Base64 PSBT file; omit or use - for stdin.
        #[arg(short, long)]
        psbt: Option<PathBuf>,
        #[command(flatten)]
        output: Output,
    },
    /// Show the configured signing condition for a PSBT's CTV commitment.
    GetKey {
        /// Base64 PSBT file; omit or use - for stdin.
        #[arg(short, long)]
        psbt: Option<PathBuf>,
    },
    /// Show a decoded PSBT as JSON.
    Show {
        /// Base64 PSBT file; omit or use - for stdin.
        #[arg(short, long)]
        psbt: Option<PathBuf>,
    },
    /// Serve an HD CTV emulator using the configured network.
    Server {
        /// Seed file for the emulator key.
        seed: PathBuf,
        /// Local address to bind, for example 127.0.0.1:8080.
        interface: String,
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..))]
        request_timeout_secs: u64,
        #[arg(long, default_value_t = 64, value_parser = positive_usize)]
        max_connections: usize,
    },
}

fn positive_usize(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| "must be a positive integer".into())
}

#[derive(Subcommand)]
pub(crate) enum Psbt {
    /// Finalize a base64 PSBT and return the decoded PSBT/transaction JSON.
    Finalize {
        /// Base64 PSBT file; omit or use - for stdin.
        #[arg(long)]
        psbt: Option<PathBuf>,
        #[command(flatten)]
        output: Output,
    },
}

#[derive(Args)]
pub(crate) struct ModuleOptions {
    /// Local workspace; cached modules are stored in its modules/ directory.
    #[arg(short, long)]
    pub workspace: Option<PathBuf>,
    /// Network for module metadata; create instead uses its input context.network.
    #[arg(long)]
    pub network: Option<Network>,
    /// JSON object mapping dependency aliases to module hashes.
    #[arg(long)]
    pub plugin_map: Option<PathBuf>,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
pub(crate) struct ModuleSource {
    /// WASM contract module file.
    #[arg(short, long)]
    pub file: Option<PathBuf>,
    /// Cached WASM module hash.
    #[arg(short, long)]
    pub key: Option<String>,
}

#[derive(Args)]
pub(crate) struct Module {
    #[command(flatten)]
    pub options: ModuleOptions,
    #[command(flatten)]
    pub source: ModuleSource,
    #[command(flatten)]
    pub output: Output,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
pub(crate) struct Funding {
    /// Fetch the specified TXID:VOUT from the configured Bitcoin node.
    #[arg(long)]
    pub outpoint: Option<OutPoint>,
    /// Funding PSBT file, including its finalized scriptSigs when applicable.
    #[arg(long)]
    pub funding_psbt: Option<PathBuf>,
    /// Create a synthetic funding transaction for a local demonstration.
    #[arg(long)]
    pub mock: bool,
    /// Ask the configured Bitcoin wallet to fund the contract.
    #[arg(long)]
    pub wallet_fund: bool,
}

#[derive(Subcommand)]
pub(crate) enum Contract {
    /// Compile a WASM module into a directly usable artifact JSON.
    Create {
        #[command(flatten)]
        module: Module,
        /// CreateArgs JSON file; omit or use - for stdin. Contains public lowering/network inputs.
        #[arg(long)]
        args: Option<PathBuf>,
    },
    /// Load a WASM module into the local cache and return its hash.
    Load {
        #[command(flatten)]
        options: ModuleOptions,
        #[arg(short, long)]
        file: PathBuf,
        #[command(flatten)]
        output: Output,
    },
    /// Inspect a module's JSON API without runtime configuration.
    Api(Module),
    /// Inspect module name and description without runtime configuration.
    Info(Module),
    /// Return a module's base64 PNG logo as a JSON string.
    Logo(Module),
    /// List locally cached modules without runtime configuration.
    List {
        #[command(flatten)]
        options: ModuleOptions,
        #[command(flatten)]
        output: Output,
    },
    /// Bind an artifact using an explicitly configured covenant backend.
    Bind {
        /// Compiled artifact JSON file; omit or use - for stdin.
        #[arg(long)]
        artifact: Option<PathBuf>,
        #[command(flatten)]
        funding: Funding,
        #[command(flatten)]
        output: Output,
    },
    /// Validate and explain an artifact without runtime configuration.
    Explain(Explain),
    /// Prepare, resume and complete an explicitly selected spend.
    Spend {
        #[command(subcommand)]
        command: Spend,
    },
}

#[derive(Args)]
pub(crate) struct Explain {
    /// Compiled artifact JSON file; omit or use - for stdin.
    #[arg(short, long)]
    pub file: Option<PathBuf>,
    /// Optional base64 PSBT file.
    #[arg(long)]
    pub psbt: Option<PathBuf>,
    /// Optional JSON capability inventory, without private keys or evidence bytes.
    #[arg(long)]
    pub assets: Option<PathBuf>,
    /// PSBT input to inspect; defaults to zero.
    #[arg(long, requires = "psbt")]
    pub input: Option<usize>,
    /// Return the complete portable explanation as JSON.
    #[arg(short, long)]
    pub json: bool,
    #[command(flatten)]
    pub output: Output,
}

#[derive(Args)]
pub(crate) struct Resume {
    /// Trusted compiled artifact JSON.
    #[arg(long)]
    pub artifact: PathBuf,
    /// Immutable prepared spend JSON.
    #[arg(long)]
    pub intent: PathBuf,
    /// Current base64 PSBT; omit to use the original baseline.
    #[arg(long)]
    pub psbt: Option<PathBuf>,
    #[command(flatten)]
    pub output: Output,
}

#[derive(Clone, Debug)]
pub(crate) struct IndexedResponse {
    pub index: usize,
    pub file: PathBuf,
}

fn response(value: &str) -> Result<IndexedResponse, String> {
    let (index, file) = value.split_once('=').ok_or("expected INDEX=FILE")?;
    if file.is_empty() {
        return Err("response file must not be empty".into());
    }
    Ok(IndexedResponse {
        index: index
            .parse()
            .map_err(|_| "request index must be a nonnegative integer")?,
        file: file.into(),
    })
}

fn spend_path(value: &str) -> Result<SpendPath, String> {
    match value {
        "key" => Ok(SpendPath::KeyPath),
        "descriptor" => Ok(SpendPath::Descriptor),
        _ => value
            .strip_prefix("script:")
            .ok_or_else(|| "expected key, descriptor, or script:<leaf hash>".into())
            .and_then(|hash| {
                hash.parse()
                    .map(SpendPath::ScriptPath)
                    .map_err(|error| format!("invalid leaf hash: {error}"))
            }),
    }
}

#[derive(Subcommand)]
pub(crate) enum Spend {
    /// Freeze a selected branch and its independent program requests.
    Prepare {
        #[arg(long)]
        artifact: PathBuf,
        /// Funded base64 PSBT file.
        #[arg(long)]
        psbt: PathBuf,
        /// Explicit branch: key, descriptor, or script:<leaf hash>.
        #[arg(long, value_parser = spend_path)]
        path: SpendPath,
        #[arg(long, default_value_t = 0)]
        input: usize,
        /// JSON SpendAssets capability inventory; defaults to empty.
        #[arg(long)]
        assets: Option<PathBuf>,
        /// JSON array of exact ProgramEvidence values; defaults to empty.
        #[arg(long)]
        evidence: Option<PathBuf>,
        #[command(flatten)]
        output: Output,
        /// Also save the prepared baseline in a new base64 PSBT file.
        #[arg(long)]
        psbt_output: Option<PathBuf>,
    },
    /// Export original requests with stable zero-based indexes.
    Requests {
        #[command(flatten)]
        resume: Resume,
        /// Export only this original request, without the index wrapper.
        #[arg(long)]
        index: Option<usize>,
    },
    /// Verify and merge program responses in any order.
    Apply {
        #[command(flatten)]
        resume: Resume,
        /// Request index and response PSBT file: INDEX=FILE; may be repeated.
        #[arg(long, required = true, value_parser = response)]
        response: Vec<IndexedResponse>,
    },
    /// Revalidate the intent and report its current requirements.
    Status(Resume),
    /// Sign only native slots selected by this intent.
    SignNative {
        #[command(flatten)]
        resume: Resume,
        /// Explicit binary Xpriv key file.
        #[arg(long)]
        key: PathBuf,
    },
    /// Complete the selected witness and check retained funding rules.
    Finalize {
        #[command(flatten)]
        resume: Resume,
        /// Output checked transaction hex instead of a finalized base64 PSBT.
        #[arg(long)]
        transaction: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn command_tree_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn typed_arguments_reject_ignored_values_and_ambiguous_funding() {
        for args in [
            vec!["sapio-cli", "signer", "show", "--input", "one", "two"],
            vec![
                "sapio-cli",
                "studio",
                "server",
                "--interface",
                "127.0.0.1:0",
            ],
            vec!["sapio-cli", "contract", "bind", "--mock", "--wallet-fund"],
            vec!["sapio-cli", "contract", "bind"],
            vec![
                "sapio-cli",
                "contract",
                "create",
                "--file",
                "one",
                "--key",
                "two",
            ],
            vec!["sapio-cli", "contract", "explain", "--input", "0"],
        ] {
            assert!(Cli::try_parse_from(args).is_err());
        }
        let outpoint = format!("{}:2", "00".repeat(32));
        let parsed =
            Cli::try_parse_from(["sapio-cli", "contract", "bind", "--outpoint", &outpoint])
                .unwrap();
        let Command::Contract {
            command: Contract::Bind { funding, .. },
        } = parsed.command
        else {
            panic!("bind command")
        };
        assert_eq!(funding.outpoint.unwrap().vout, 2);
    }
}
