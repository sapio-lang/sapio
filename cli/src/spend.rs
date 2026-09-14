//! File-based completion of an explicitly selected contract spend.

use bitcoin::psbt::Psbt;
use bitcoin::secp256k1::Secp256k1;
use clap::{App, AppSettings, Arg, ArgMatches};
use emulator_connect::program::completion::SpendIntent;
use emulator_connect::program::{prepare_spend, ProgramEvidence, SpendAssets, SpendPath};
use sapio::contract::Compiled;
use serde::{de::DeserializeOwned, Serialize};
use std::error::Error;
use std::fs::OpenOptions;
use std::io::Write;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn file(name: &'static str, help: &'static str) -> Arg<'static> {
    Arg::new(name).long(name).takes_value(true).about(help)
}

fn resume(command: App<'static>) -> App<'static> {
    command
        .arg(file("artifact", "Trusted compiled artifact JSON").required(true))
        .arg(file("intent", "Immutable prepared spend JSON").required(true))
        .arg(file(
            "psbt",
            "Current base64 PSBT; omit to use the intent's original baseline",
        ))
        .arg(file(
            "output",
            "Write a new file; omit for stdout; existing files are never overwritten",
        ))
}

pub(crate) fn command() -> App<'static> {
    App::new("spend")
        .about("Prepare, resume and finish one selected spend without network configuration")
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .subcommand(
            App::new("prepare")
                .about("Freeze a selected branch and its independent program requests")
                .arg(file("artifact", "Trusted compiled artifact JSON").required(true))
                .arg(file("psbt", "Funded base64 PSBT").required(true))
                .arg(
                    Arg::new("path")
                        .long("path")
                        .takes_value(true)
                        .required(true)
                        .about("Explicit branch: key, descriptor, or script:<leaf hash>"),
                )
                .arg(
                    Arg::new("input")
                        .long("input")
                        .takes_value(true)
                        .default_value("0"),
                )
                .arg(file(
                    "assets",
                    "JSON SpendAssets capability inventory; defaults to empty",
                ))
                .arg(file(
                    "evidence",
                    "JSON array of exact ProgramEvidence values; defaults to empty",
                ))
                .arg(file(
                    "output",
                    "Write intent JSON to a new file; omit for stdout",
                ))
                .arg(file(
                    "psbt-output",
                    "Optionally save the prepared baseline as a new base64 PSBT file",
                )),
        )
        .subcommand(
            resume(
                App::new("requests")
                    .about("Export original requests with stable zero-based indexes"),
            )
            .arg(
                Arg::new("index")
                    .long("index")
                    .takes_value(true)
                    .about("Export only this ProgramSigningRequest, without the index wrapper"),
            ),
        )
        .subcommand(
            resume(App::new("apply").about("Verify and merge program responses in any order")).arg(
                Arg::new("response")
                    .long("response")
                    .takes_value(true)
                    .multiple(true)
                    .number_of_values(1)
                    .required(true)
                    .about("Request index and response PSBT file: INDEX=FILE; may be repeated"),
            ),
        )
        .subcommand(resume(
            App::new("status").about("Revalidate the intent and report current requirements"),
        ))
        .subcommand(
            resume(App::new("sign-native").about("Sign only native slots selected by this intent"))
                .arg(
                    file("key", "Explicit binary Xpriv key file used by signer sign")
                        .required(true),
                ),
        )
        .subcommand(
            resume(
                App::new("finalize")
                    .about("Complete the selected witness and check retained funding rules"),
            )
            .arg(
                Arg::new("transaction")
                    .long("transaction")
                    .about("Output checked transaction hex instead of the finalized base64 PSBT"),
            ),
        )
}

fn json<T: DeserializeOwned>(path: &str) -> Result<T> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

fn psbt(path: &str) -> Result<Psbt> {
    let encoded = std::fs::read_to_string(path)?;
    Ok(Psbt::deserialize(&base64::decode(encoded.trim())?)?)
}

fn output(path: Option<&str>, bytes: &[u8]) -> Result<()> {
    match path {
        Some(path) => {
            let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(bytes)?;
            file.write_all(b"\n")?;
        }
        None => {
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(bytes)?;
            stdout.write_all(b"\n")?;
        }
    }
    Ok(())
}

fn output_json(path: Option<&str>, value: &impl Serialize) -> Result<()> {
    output(path, &serde_json::to_vec_pretty(value)?)
}

fn output_psbt(path: Option<&str>, psbt: &Psbt) -> Result<()> {
    output(path, base64::encode(psbt.serialize()).as_bytes())
}

fn parse_path(path: &str) -> Result<SpendPath> {
    match path {
        "key" => Ok(SpendPath::KeyPath),
        "descriptor" => Ok(SpendPath::Descriptor),
        _ => path
            .strip_prefix("script:")
            .ok_or_else(|| "--path must be key, descriptor, or script:<leaf hash>".into())
            .and_then(|hash| Ok(SpendPath::ScriptPath(hash.parse()?))),
    }
}

pub(crate) fn run(matches: &ArgMatches) -> Result<()> {
    let (operation, args) = matches.subcommand().ok_or("missing spend operation")?;
    let artifact: Compiled = json(args.value_of("artifact").unwrap())?;
    if operation == "prepare" {
        let psbt = psbt(args.value_of("psbt").unwrap())?;
        let assets: SpendAssets = args
            .value_of("assets")
            .map(json)
            .transpose()?
            .unwrap_or_default();
        let evidence: Vec<ProgramEvidence> = args
            .value_of("evidence")
            .map(json)
            .transpose()?
            .unwrap_or_default();
        let prepared = prepare_spend(
            &artifact,
            parse_path(args.value_of("path").unwrap())?,
            psbt,
            args.value_of("input").unwrap().parse()?,
            &assets,
            &evidence,
        )?;
        let intent = SpendIntent::from_prepared(prepared);
        if let Some(path) = args.value_of("psbt-output") {
            output_psbt(Some(path), intent.baseline_psbt())?;
        }
        return output_json(args.value_of("output"), &intent);
    }

    let intent: SpendIntent = json(args.value_of("intent").unwrap())?;
    let mut current = args
        .value_of("psbt")
        .map(psbt)
        .transpose()?
        .unwrap_or_else(|| intent.baseline_psbt().clone());
    // Even request export validates persisted intent against the trusted artifact.
    let status = intent.status(&artifact, &current)?;
    let destination = args.value_of("output");
    match operation {
        "status" => output_json(destination, &status),
        "requests" => {
            if let Some(index) = args.value_of("index") {
                let index: usize = index.parse()?;
                let request = intent
                    .requests()
                    .get(index)
                    .ok_or("request index is out of range")?;
                output_json(destination, request)
            } else {
                let requirements = intent.request_requirements(&artifact)?;
                let requests: Vec<_> = intent
                    .requests()
                    .iter()
                    .zip(requirements)
                    .enumerate()
                    .map(|(index, (request, requirement))| serde_json::json!({"index": index, "requirement": requirement, "request": request}))
                    .collect();
                output_json(destination, &requests)
            }
        }
        "apply" => {
            for response in args.values_of("response").unwrap() {
                let (index, path) = response
                    .split_once('=')
                    .ok_or("--response requires INDEX=FILE")?;
                intent.merge_response(&artifact, &mut current, index.parse()?, &psbt(path)?)?;
            }
            output_psbt(destination, &current)
        }
        "sign-native" => {
            let key = sapio_psbt::SigningKey::read_key_from_buf(&std::fs::read(
                args.value_of("key").unwrap(),
            )?)?;
            intent.sign_native(&artifact, &mut current, &key, &Secp256k1::new())?;
            output_psbt(destination, &current)
        }
        "finalize" => {
            let finalized = intent.finalize(&artifact, &current, &Secp256k1::new())?;
            if args.is_present("transaction") {
                output(
                    destination,
                    bitcoin::consensus::encode::serialize_hex(&finalized.extract_tx()?).as_bytes(),
                )
            } else {
                output_psbt(destination, &finalized)
            }
        }
        _ => unreachable!("clap restricts spend operations"),
    }
}
