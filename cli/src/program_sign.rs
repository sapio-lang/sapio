//! Explicit local signing through an exact WASM program evaluator.

use crate::util::{read_input, read_json, write_psbt};
use bitcoin::bip32::Xpriv;
use clap::Args;
use emulator_connect::program::{ProgramOracle, ProgramSigningRequest, WasmEvaluator};
use sapio_base::program::{EvaluatorId, WasmVersion, MAX_PROGRAM_BYTES};
use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

#[derive(Debug, Args)]
pub(crate) struct ProgramSignArgs {
    /// Explicit binary Xpriv file for this program's oracle.
    #[arg(long)]
    key: PathBuf,
    /// Exported ProgramSigningRequest JSON file, or '-' for stdin.
    #[arg(long)]
    request: PathBuf,
    /// Exact interpreter WASM for a registered evaluator; omit for inline WASM.
    #[arg(long)]
    evaluator: Option<PathBuf>,
    /// Write the response PSBT to a new file; omit for stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,
}

pub(crate) fn run(args: ProgramSignArgs) -> Result<(), Box<dyn Error>> {
    let request: ProgramSigningRequest = read_json(Some(&args.request))?;
    let identity = request.instance.evaluator();
    let evaluators = match (identity.inline_wasm_version(), args.evaluator.as_deref()) {
        (Some(_), None) => Vec::new(),
        (Some(_), Some(_)) => {
            return Err("inline WASM requests carry their evaluator; omit --evaluator".into());
        }
        (None, None) => {
            return Err(format!(
                "evaluator {} requires an explicit --evaluator WASM file",
                identity.0
            )
            .into());
        }
        (None, Some(path)) => {
            let mut module = Vec::new();
            File::open(path)?
                .take(MAX_PROGRAM_BYTES as u64 + 1)
                .read_to_end(&mut module)?;
            if module.len() > MAX_PROGRAM_BYTES {
                return Err("evaluator WASM exceeds the 65,536-byte program limit".into());
            }
            // The request commits its ABI as part of the evaluator identity.
            // Matching that identity does not try a different execution rule.
            let version = [WasmVersion::V1, WasmVersion::V2]
                .into_iter()
                .find(|version| EvaluatorId::for_wasm_version(&module, *version) == identity)
                .ok_or("evaluator bytes do not match the request's committed evaluator ID")?;
            vec![WasmEvaluator::with_version(version, module)?]
        }
    };
    let root = Xpriv::decode(&read_input(Some(&args.key))?)?;
    let response = ProgramOracle::new(root, evaluators)?.sign(request)?;
    write_psbt(args.output.as_deref(), &response)
}
