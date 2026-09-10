// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! configuration file format / parsing for sapio command line interface

use bitcoin::util::bip32::ExtendedPubKey;
use bitcoincore_rpc_async as rpc;

use directories::BaseDirs;
use emulator_connect::connections::federated::FederatedEmulatorConnection;
use emulator_connect::connections::hd::HDOracleEmulatorConnection;
use emulator_connect::{CTVAvailable, CTVEmulator};
use schemars::JsonSchema;
use serde::*;
use std::collections::BTreeMap;
use std::convert::TryFrom;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::BufReader;

#[cfg(test)]
mod tests;

/// Runtime signer peers used for binding and signing already compiled policies.
/// Public compilation inputs are supplied separately in `context.lowering`.
#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EmulatorConfig {
    /// list of emulators to use & how to contact them
    #[schemars(with = "Vec<(String, String)>")]
    pub emulators: Vec<(ExtendedPubKey, String)>,
    /// threshold could be larger than u8, but that seems very unlikely/an error.
    pub threshold: u8,
    /// Elapsed seconds allowed for configuration resolution and each signing
    /// request, including time waiting for this connection's previous request.
    #[serde(default = "default_request_timeout_secs")]
    pub request_timeout_secs: u64,
}

/// Runtime covenant enforcement assumption used for binding and signing.
/// Selecting native CTV does not detect or establish a node's consensus rules.
#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum CovenantConfig {
    /// Bind scripts on a research chain assumed by the operator to enforce native CTV.
    NativeCtvResearch {},
    /// Use configured signers to satisfy previously lowered CTV policies.
    SignerEmulation(EmulatorConfig),
    /// Use signer emulation while also assuming native CTV for direct script checks.
    SignerEmulationWithNativeCtvResearch(EmulatorConfig),
}

impl CovenantConfig {
    /// Resolve the explicitly selected backend; errors never select another mode.
    pub async fn get_emulator(&self) -> Result<Arc<dyn CTVEmulator>, Box<dyn std::error::Error>> {
        match self {
            Self::NativeCtvResearch {} => Ok(Arc::new(CTVAvailable)),
            Self::SignerEmulation(config) | Self::SignerEmulationWithNativeCtvResearch(config) => {
                config.get_emulator().await
            }
        }
    }

    /// Whether the operator explicitly assumes native CTV enforcement on the chain.
    pub fn allows_native_ctv(&self) -> bool {
        matches!(
            self,
            Self::NativeCtvResearch {} | Self::SignerEmulationWithNativeCtvResearch(_)
        )
    }
}

fn default_request_timeout_secs() -> u64 {
    emulator_connect::DEFAULT_REQUEST_TIMEOUT.as_secs()
}

impl EmulatorConfig {
    /// Resolve the configured peers without opening signing connections.
    /// Waiting for resolution has one deadline across the whole configuration.
    /// The system's blocking DNS work can outlive this wait and delay shutdown.
    pub async fn get_emulator(&self) -> Result<Arc<dyn CTVEmulator>, Box<dyn std::error::Error>> {
        sapio_base::covenant::LoweringPlan::CtvEmulation {
            signers: self.emulators.iter().map(|(key, _)| *key).collect(),
            threshold: self.threshold,
        }
        .validate()?;
        let timeout = Duration::from_secs(self.request_timeout_secs);
        let deadline = tokio::time::Instant::now()
            .checked_add(timeout)
            .filter(|_| !timeout.is_zero())
            .ok_or("Emulator request timeout must be positive and representable")?;
        tokio::time::timeout_at(deadline, async {
            let secp = Arc::new(bitcoin::secp256k1::Secp256k1::new());
            let mut peers: Vec<Arc<dyn CTVEmulator>> = Vec::with_capacity(self.emulators.len());
            for (root, host) in &self.emulators {
                peers.push(Arc::new(
                    HDOracleEmulatorConnection::new(host.as_str(), *root, None, secp.clone())
                        .await?
                        .with_request_timeout(timeout)?,
                ));
            }
            Ok(if peers.len() == 1 {
                peers.pop().expect("one peer")
            } else {
                Arc::new(FederatedEmulatorConnection::new(peers, self.threshold))
            })
        })
        .await
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Emulator configuration resolution timed out",
            )
        })?
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(into = "PathBuf")]
#[serde(from = "String")]
struct PathBufWrapped(PathBuf);
impl From<String> for PathBufWrapped {
    fn from(s: String) -> Self {
        PathBufWrapped(s.into())
    }
}
impl Into<PathBuf> for PathBufWrapped {
    fn into(self) -> PathBuf {
        self.0
    }
}
/// Used to serailize/deserialize pathbufs for config
mod pathbuf_serde {
    use serde::*;
    use std::path::PathBuf;
    pub fn serialize<S>(p: &PathBuf, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        s.serialize_str(p.to_str().unwrap())
    }
    pub fn deserialize<'de, D>(d: D) -> Result<PathBuf, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(String::deserialize(d)?.into())
    }
}

/// Remote type Derivation for rpc::Auth
/// TODO: Move to the RPC Library?
#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
#[serde(remote = "rpc::Auth")]
pub enum Auth {
    /// No Auth Used
    None,
    /// Username and Passowrd
    UserPass(String, String),
    /// Cookie File
    CookieFile(
        #[serde(with = "pathbuf_serde")]
        #[schemars(with = "String")]
        PathBuf,
    ),
}

/// Which Bitcoin Node should Sapio use
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Node {
    /// the url to connect to
    pub url: String,
    /// the auth to use
    #[serde(with = "Auth")]
    pub auth: rpc::Auth,
}

/// A configuration for any network (regtest, main, signet, testnet)
/// Only one config may set active = true at a time.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct NetworkConfig {
    /// if this is the active config
    pub active: bool,
    /// the node to connect to
    pub api_node: Node,
    /// The operator's explicit covenant enforcement choice.
    pub covenant: CovenantConfig,
    /// mapping of name:module hash for translation during compilation
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub plugin_map: Option<BTreeMap<String, WasmerCacheHash>>,
}

impl From<WasmerCacheHash> for [u8; 32] {
    fn from(x: WasmerCacheHash) -> Self {
        x.0
    }
}

/// An ID for an uncompiled Plugin Wasm Binary
/// It is serialized as a hex slice.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(try_from = "String", into = "String")]
pub struct WasmerCacheHash([u8; 32]);

use bitcoin::hashes::hex::{FromHex, ToHex};
impl From<WasmerCacheHash> for String {
    fn from(x: WasmerCacheHash) -> Self {
        ToHex::to_hex(&x.0[..])
    }
}

impl TryFrom<String> for WasmerCacheHash {
    type Error = bitcoin::hashes::hex::Error;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        FromHex::from_hex(&s).map(WasmerCacheHash)
    }
}

/// This config has only the currently active network, the other configs get
/// dropped during the ConfigVerifier::try_into.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(try_from = "ConfigVerifier")]
pub struct Config {
    /// the currently active configuration
    pub active: NetworkConfig,
    /// which network the configuration is for
    pub network: bitcoin::network::constants::Network,
}

impl Config {
    /// reads the user's config file and returns it,
    /// or a different one if the user specified a different file manually.
    ///
    /// if no config is found for the user, creates a file.
    ///
    /// **Race Conditions** This is clearly not safe if multiple edits are
    /// happening on config.json. It is assumed that the user will ensure
    /// writes to config.json are safe.
    pub async fn setup(
        custom_config: Option<&str>,
        typ: &str,
        org: &str,
        proj: &str,
    ) -> Result<Config, Box<dyn std::error::Error>> {
        if let Some(p) = custom_config {
            Ok(serde_json::from_slice(&tokio::fs::read(p).await?[..])?)
        } else {
            let proj = directories::ProjectDirs::from(typ, org, proj)
                .expect("Failed to find config directory");
            let path = proj.config_dir();
            tokio::fs::create_dir_all(path).await?;
            let mut pb = path.to_path_buf();
            pb.push("config.json");
            if let Ok(txt) = tokio::fs::read(&pb).await {
                Ok(serde_json::from_slice(&txt[..])?)
            } else {
                Err("Please Run the configure wizard command to make a config file")?
            }
        }
    }
}

/// This is a deserialization helper which checks the config file for well
/// formedness before processing into an actual config.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConfigVerifier {
    main: Option<NetworkConfig>,
    testnet: Option<NetworkConfig>,
    signet: Option<NetworkConfig>,
    regtest: Option<NetworkConfig>,
}
impl TryFrom<ConfigVerifier> for Config {
    type Error = ConfigError;

    fn try_from(
        cfg: ConfigVerifier,
    ) -> std::result::Result<Self, <Self as std::convert::TryFrom<ConfigVerifier>>::Error> {
        let network = cfg.get_network()?;
        Ok(Config {
            active: cfg.check()?,
            network,
        })
    }
}

impl From<Config> for ConfigVerifier {
    fn from(c: Config) -> ConfigVerifier {
        let mut res = ConfigVerifier {
            main: None,
            testnet: None,
            signet: None,
            regtest: None,
        };
        match c.network {
            bitcoin::network::constants::Network::Regtest => {
                res.regtest = Some(c.active);
            }
            bitcoin::network::constants::Network::Signet => {
                res.signet = Some(c.active);
            }
            bitcoin::network::constants::Network::Testnet => {
                res.testnet = Some(c.active);
            }
            bitcoin::network::constants::Network::Bitcoin => {
                res.main = Some(c.active);
            }
        };
        res
    }
}

impl ConfigVerifier {
    /// Return the active network
    fn get_network(&self) -> Result<bitcoin::network::constants::Network, ConfigError> {
        match self.get_n() {
            1 => Err(ConfigError::NoActiveConfig),
            3 => Ok(bitcoin::network::constants::Network::Bitcoin),
            11 => Ok(bitcoin::network::constants::Network::Testnet),
            7 => Ok(bitcoin::network::constants::Network::Regtest),
            5 => Ok(bitcoin::network::constants::Network::Signet),
            _ => Err(ConfigError::TooManyActiveNetworks),
        }
    }
    /// This is some... clever... code which assigns a prime number to every
    /// active network and then multiplies them all together.
    ///
    /// The result can then be used to pick which network should be used & verify
    /// that only one network is active at once.
    ///
    /// The alternative is a bit messier unfortunately, but maybe simpler as a refactor.
    fn get_n(&self) -> i32 {
        let v0 = self.main.as_ref().map(|c| 3 * c.active as i32).unwrap_or(1);
        let v1 = self
            .signet
            .as_ref()
            .map(|c| 5 * c.active as i32)
            .unwrap_or(1);
        let v2 = self
            .regtest
            .as_ref()
            .map(|c| 7 * c.active as i32)
            .unwrap_or(1);
        let v3 = self
            .testnet
            .as_ref()
            .map(|c| 11 * c.active as i32)
            .unwrap_or(1);
        v0 * v1 * v2 * v3
    }
    /// Checks the config for correctness and then returns the active config.
    pub fn check(self) -> Result<NetworkConfig, ConfigError> {
        match self.get_n() {
            1 => Err(ConfigError::NoActiveConfig),
            3 => Ok(self.main.unwrap()),
            5 => Ok(self.signet.unwrap()),
            7 => Ok(self.regtest.unwrap()),
            11 => Ok(self.testnet.unwrap()),
            _ => Err(ConfigError::TooManyActiveNetworks),
        }
    }

    /// a setup wizard to generate a new config file
    pub async fn wizard() -> Result<Self, Box<dyn std::error::Error>> {
        use tokio::io::AsyncBufReadExt;

        let stdin = tokio::io::stdin();
        let reader = BufReader::new(stdin);
        let mut b = BaseDirs::new()
            .expect("Could Not Determine a Base Directory")
            .home_dir()
            .to_path_buf();
        b.push(".bitcoin");
        b.push("regtest");
        b.push(".cookie");
        let network;
        let mut lines = reader.lines();
        loop {
            println!("Which Network? (main, reg, sig, test): ");
            if let Some(line) = lines.next_line().await? {
                network = match line.trim() {
                    "main" => bitcoin::network::constants::Network::Bitcoin,
                    "reg" => bitcoin::network::constants::Network::Regtest,
                    "sig" => bitcoin::network::constants::Network::Signet,
                    "test" => bitcoin::network::constants::Network::Testnet,
                    _ => {
                        println!("Not a valid option {:?}", line);
                        continue;
                    }
                };
                break;
            }
        }
        let mut url: String;
        loop {
            println!("API Node URL (e.g., http://127.0.0.1:18443): ");
            if let Some(line) = lines.next_line().await? {
                url = line.trim().into();
                if url.is_empty() {
                    println!("Must enter a username");
                } else {
                    break;
                }
            }
        }
        let using_cookie;
        loop {
            println!("Auth Type (for cookie file, \"cookie\", for username/password \"basic\"): ");
            if let Some(line) = lines.next_line().await? {
                using_cookie = match line.trim() {
                    "cookie" => true,
                    "basic" => false,
                    l => {
                        println!("Invalid option {}, type cookie or basic:", l);
                        continue;
                    }
                };
                break;
            }
        }
        let auth = if using_cookie {
            let mut cookie: String;
            loop {
                println!("Cookie file location (e.g., {}): ", b.display());
                if let Some(line) = lines.next_line().await? {
                    cookie = line.trim().into();
                    if cookie.is_empty() {
                        println!("Must give a cookie file location.");
                        continue;
                    }
                    break;
                }
            }
            rpc::Auth::CookieFile(cookie.into())
        } else {
            let mut username: String;
            loop {
                println!("Username: ");
                if let Some(line) = lines.next_line().await? {
                    username = line.trim().into();
                    if username.is_empty() {
                        println!("Must enter a username");
                    } else {
                        break;
                    }
                }
            }
            let mut password: String;
            loop {
                println!("Password: ");
                if let Some(line) = lines.next_line().await? {
                    password = line.trim().into();
                    if password.is_empty() {
                        println!("Must enter a username");
                    } else {
                        break;
                    }
                }
            }
            rpc::Auth::UserPass(username, password)
        };

        let covenant = covenant_wizard(&mut lines).await?;
        println!("Configuration Complete!");
        println!("To configure plugin maps, edit the configuration manually.");
        println!("Your Configuration:");

        let active = NetworkConfig {
            active: true,
            api_node: Node { url, auth },
            covenant,
            plugin_map: None,
        };
        let cv: ConfigVerifier = Config { network, active }.into();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::to_value(cv.clone())?)?
        );

        Ok(cv)
    }
}

/// Errors that can arise when validating a configuration file
#[derive(Debug)]
pub enum ConfigError {
    /// Only one network can be active at a time
    TooManyActiveNetworks,
    /// One network must be active
    NoActiveConfig,
}
use std::fmt;
impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for ConfigError {}

async fn covenant_wizard<R: tokio::io::AsyncBufRead + Unpin>(
    lines: &mut tokio::io::Lines<R>,
) -> Result<CovenantConfig, Box<dyn std::error::Error>> {
    let native_ctv_research = loop {
        println!("Covenant mode (native_ctv_research / signer_emulation / signer_emulation_with_native_ctv_research):");
        println!(
            "native_ctv_research assumes your chain enforces CTV; Sapio does not detect this."
        );
        println!("signer_emulation relies on the configured signers' security and availability.");
        println!("signer_emulation_with_native_ctv_research combines both assumptions for mixed scripts.");
        let line = lines.next_line().await?.ok_or("Missing covenant mode")?;
        match line.trim() {
            "native_ctv_research" => return Ok(CovenantConfig::NativeCtvResearch {}),
            "signer_emulation" => break false,
            "signer_emulation_with_native_ctv_research" => break true,
            _ => println!("Choose one of the three explicit covenant modes."),
        }
    };

    let mut emulators = Vec::new();
    loop {
        println!("Signer extended public key (blank to finish after adding a signer):");
        let line = lines
            .next_line()
            .await?
            .ok_or("Missing signer public key")?;
        let key = line.trim();
        if key.is_empty() && !emulators.is_empty() {
            break;
        }
        let key = match ExtendedPubKey::from_str(key) {
            Ok(key) => key,
            Err(error) => {
                println!("Invalid extended public key: {error}");
                continue;
            }
        };
        let address = loop {
            println!("Signer address (host:port):");
            let line = lines.next_line().await?.ok_or("Missing signer address")?;
            if !line.trim().is_empty() {
                break line.trim().to_owned();
            }
        };
        emulators.push((key, address));
    }
    let threshold = loop {
        println!("Required signer threshold (1..={}):", emulators.len());
        let line = lines.next_line().await?.ok_or("Missing signer threshold")?;
        match line.trim().parse::<u8>() {
            Ok(threshold) if threshold > 0 && usize::from(threshold) <= emulators.len() => {
                break threshold;
            }
            _ => println!("Threshold must be positive and no greater than the signer count."),
        }
    };
    let config = EmulatorConfig {
        emulators,
        threshold,
        request_timeout_secs: default_request_timeout_secs(),
    };
    Ok(if native_ctv_research {
        CovenantConfig::SignerEmulationWithNativeCtvResearch(config)
    } else {
        CovenantConfig::SignerEmulation(config)
    })
}
