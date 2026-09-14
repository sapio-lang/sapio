//! File-based completion of an explicitly selected contract spend.

use crate::args::Spend;
use crate::util::{read_input, read_json, read_psbt, write_json, write_output, write_psbt, Result};
use bitcoin::secp256k1::Secp256k1;
use emulator_connect::program::completion::SpendIntent;
use emulator_connect::program::{prepare_spend, ProgramEvidence, SpendAssets};
use sapio::contract::Compiled;

pub(crate) fn run(command: Spend) -> Result<()> {
    if let Spend::Prepare {
        artifact,
        psbt,
        path,
        input,
        assets,
        evidence,
        output,
        psbt_output,
    } = command
    {
        let artifact: Compiled = read_json(Some(&artifact))?;
        let assets: SpendAssets = assets
            .as_deref()
            .map(|path| read_json(Some(path)))
            .transpose()?
            .unwrap_or_default();
        let evidence: Vec<ProgramEvidence> = evidence
            .as_deref()
            .map(|path| read_json(Some(path)))
            .transpose()?
            .unwrap_or_default();
        let prepared = prepare_spend(
            &artifact,
            path,
            read_psbt(Some(&psbt))?,
            input,
            &assets,
            &evidence,
        )?;
        let intent = SpendIntent::from_prepared(prepared);
        if let Some(path) = psbt_output {
            write_psbt(Some(&path), intent.baseline_psbt())?;
        }
        return write_json(output.output.as_deref(), &intent);
    }

    let resume = match &command {
        Spend::Requests { resume, .. }
        | Spend::Apply { resume, .. }
        | Spend::Status(resume)
        | Spend::SignNative { resume, .. }
        | Spend::Finalize { resume, .. } => resume,
        Spend::Prepare { .. } => unreachable!("prepare returned above"),
    };
    let artifact: Compiled = read_json(Some(&resume.artifact))?;
    let intent: SpendIntent = read_json(Some(&resume.intent))?;
    let mut current = resume
        .psbt
        .as_deref()
        .map(|path| read_psbt(Some(path)))
        .transpose()?
        .unwrap_or_else(|| intent.baseline_psbt().clone());
    // Request export also validates the persisted intent against its trusted artifact.
    let status = intent.status(&artifact, &current)?;
    let output = resume.output.output.clone();
    let destination = output.as_deref();
    match command {
        Spend::Status(_) => write_json(destination, &status),
        Spend::Requests {
            index: Some(index), ..
        } => {
            let request = intent
                .requests()
                .get(index)
                .ok_or("request index is out of range")?;
            write_json(destination, request)
        }
        Spend::Requests { index: None, .. } => {
            let requirements = intent.request_requirements(&artifact)?;
            let requests: Vec<_> = intent
                .requests()
                .iter()
                .zip(requirements)
                .enumerate()
                .map(|(index, (request, requirement))| {
                    serde_json::json!({
                        "index": index, "requirement": requirement, "request": request,
                    })
                })
                .collect();
            write_json(destination, &requests)
        }
        Spend::Apply { response, .. } => {
            for response in response {
                intent.merge_response(
                    &artifact,
                    &mut current,
                    response.index,
                    &read_psbt(Some(&response.file))?,
                )?;
            }
            write_psbt(destination, &current)
        }
        Spend::SignNative { key, .. } => {
            let key = sapio_psbt::SigningKey::read_key_from_buf(&read_input(Some(&key))?)?;
            intent.sign_native(&artifact, &mut current, &key, &Secp256k1::new())?;
            write_psbt(destination, &current)
        }
        Spend::Finalize { transaction, .. } => {
            let finalized = intent.finalize(&artifact, &current, &Secp256k1::new())?;
            if transaction {
                write_output(
                    destination,
                    bitcoin::consensus::encode::serialize_hex(&finalized.extract_tx()?).as_bytes(),
                )
            } else {
                write_psbt(destination, &finalized)
            }
        }
        Spend::Prepare { .. } => unreachable!("prepare returned above"),
    }
}
