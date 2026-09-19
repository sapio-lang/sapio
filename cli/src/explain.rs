//! Read-only artifact and spending explanations without runtime configuration.

use crate::args::Explain;
use crate::util::{read_json, read_psbt, write_json, write_output};
use emulator_connect::program::spend_plan::SpendRequirement;
use emulator_connect::program::{plan_spends, SpendAssets, SpendReport};
use sapio::contract::actions::TemplateKind;
use sapio::contract::object::ArtifactExplanation;
use sapio::contract::Compiled;
use sapio_base::policy::ScriptPolicy;
use serde::Serialize;
use std::error::Error;
use std::fmt::Write as _;

#[derive(Serialize)]
struct Explanation {
    artifact: ArtifactExplanation,
    #[serde(skip_serializing_if = "Option::is_none")]
    spend: Option<SpendReport>,
}

pub(crate) fn run(args: Explain) -> Result<(), Box<dyn Error>> {
    let object: Compiled = read_json(args.file.as_deref())?;
    let artifact = object.explain()?;
    let psbt = args
        .psbt
        .as_deref()
        .map(|path| read_psbt(Some(path)))
        .transpose()?;
    let index = args.input.unwrap_or(0);
    let assets: SpendAssets = args
        .assets
        .as_deref()
        .map(|path| read_json(Some(path)))
        .transpose()?
        .unwrap_or_default();
    let spend = if psbt.is_some() || args.assets.is_some() {
        Some(plan_spends(
            &object,
            psbt.as_ref().map(|psbt| (psbt, index)),
            &assets,
        )?)
    } else {
        None
    };
    let explanation = Explanation { artifact, spend };
    if args.json {
        write_json(args.output.output.as_deref(), &explanation)?;
    } else {
        write_output(
            args.output.output.as_deref(),
            human(&explanation)?.trim_end().as_bytes(),
        )?;
    }
    Ok(())
}

fn kind(kind: TemplateKind) -> &'static str {
    match kind {
        TemplateKind::Committed => "committed",
        TemplateKind::Suggested => "suggested",
    }
}

fn policy_summary(policy: &ScriptPolicy) -> String {
    match policy {
        ScriptPolicy::Miniscript(clause) => clause.to_string(),
        ScriptPolicy::Emulatable(predicate) => format!("emulatable CTV({})", predicate.0 .0),
        ScriptPolicy::Program(program) => format!(
            "program {} via {}",
            program.instance().id().0,
            program.root()
        ),
        ScriptPolicy::Script(script) => format!(
            "raw tapscript ({} bytes; backend witness required)",
            script.as_script().len()
        ),
        ScriptPolicy::And(children) => format!(
            "all({})",
            children
                .iter()
                .map(policy_summary)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ScriptPolicy::Or(children) => format!(
            "any({})",
            children
                .iter()
                .map(policy_summary)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn human(explanation: &Explanation) -> Result<String, Box<dyn Error>> {
    let mut text = String::new();
    writeln!(
        text,
        "Validated artifact: {} output occurrences",
        explanation.artifact.nodes.len()
    )?;
    if explanation.artifact.native_ctv_in_graph {
        writeln!(
            text,
            "Known scripts assume native CTV enforcement; chain activation is not established."
        )?;
    }
    for node in &explanation.artifact.nodes {
        let path = String::from(node.source_path.0.as_ref().clone());
        let location = if node.location.is_empty() {
            "(root)"
        } else {
            &node.location
        };
        writeln!(text, "\nOutput {path} at {location}")?;
        writeln!(
            text,
            "  Destination: {}",
            serde_json::to_string(&node.address)?
        )?;
        writeln!(text, "  Required input: {} sat", node.required_input_sats)?;
        writeln!(
            text,
            "  Covenant lowering: {}",
            serde_json::to_string(&node.covenants.lowering)?
        )?;
        if let Some(descriptor) = &node.descriptor {
            writeln!(text, "  Descriptor: {}", serde_json::to_string(descriptor)?)?;
        } else {
            writeln!(
                text,
                "  Descriptor: unknown; this artifact does not describe its witnesses"
            )?;
        }
        for program in &node.program_policies {
            writeln!(
                text,
                "  Program source: {} at {:?}",
                policy_summary(&program.policy),
                program.paths
            )?;
        }
        for action in &node.actions {
            writeln!(
                text,
                "  Action {} [{}]",
                String::from(action.path.0.as_ref().clone()),
                action.kind.map(kind).unwrap_or("mode unspecified")
            )?;
            if let Some(schema) = &action.schema {
                writeln!(text, "    Request schema: {schema}")?;
            } else {
                writeln!(text, "    Local Rust callback; no JSON request schema")?;
            }
        }
        for template in &node.templates {
            writeln!(
                text,
                "  {} transaction {}",
                kind(template.kind),
                template.hash
            )?;
            writeln!(
                text,
                "    Version {}; locktime {}",
                template.version, template.lock_time
            )?;
            for input in &template.inputs {
                let name = input.name.as_deref().unwrap_or("unnamed");
                let minimum = input
                    .minimum_sats
                    .map(|amount| format!("at least {amount} sat"))
                    .unwrap_or_else(|| "contribution unknown".into());
                writeln!(
                    text,
                    "    Input {} ({name}): {minimum}; sequence {:#x}",
                    input.index, input.sequence
                )?;
            }
            for output in &template.outputs {
                writeln!(
                    text,
                    "    Output {} ({}): {} sat -> {}",
                    output.index,
                    output.name.as_deref().unwrap_or("unnamed"),
                    output.amount_sats,
                    output.script_pubkey
                )?;
            }
            writeln!(text, "    Reserved fee: {} sat", template.reserved_fee_sats)?;
            writeln!(
                text,
                "    Local fee cap: {} sat (checked against actual funding)",
                template.maximum_fee_sats
            )?;
            if let Some(funding) = &template.funding_constraints {
                if let Some(rate) = funding.minimum_feerate {
                    writeln!(
                        text,
                        "    Local minimum fee rate: {} sat/kwu; needs final witness weight",
                        rate.to_sat_per_kwu()
                    )?;
                }
            }
            for guard in &template.guards {
                writeln!(text, "    Authorization source: {}", policy_summary(guard))?;
            }
        }
    }
    if let Some(spend) = &explanation.spend {
        writeln!(text, "\nSpend planning: funding {:?}", spend.funding)?;
        if !spend.missing_prevouts.is_empty() {
            writeln!(
                text,
                "  Supply previous outputs for inputs {:?}",
                spend.missing_prevouts
            )?;
        }
        if let Some(funding) = &spend.template_funding {
            if let Some(fee) = funding.actual_fee_sats {
                writeln!(
                    text,
                    "  Actual fee: {fee} sat; reserved {} sat",
                    funding.reserved_fee_sats
                )?;
            }
            if funding.fee_rate_pending {
                writeln!(
                    text,
                    "  Finalize all input witnesses to check the requested fee rate"
                )?;
            }
        }
        for branch in &spend.branches {
            writeln!(text, "  {:?}: {:?}", branch.path, branch.status)?;
            writeln!(text, "    Policy: {}", branch.policy)?;
            writeln!(
                text,
                "    Transaction compatibility: {:?}",
                branch.transaction_compatible
            )?;
            for requirement in &branch.requirements {
                if let SpendRequirement::Program {
                    requirement,
                    signature,
                    evidence,
                    signer_available,
                    codec,
                } = requirement
                {
                    writeln!(text, "    Program {} via {}: signature {signature:?}, evidence {evidence:?}, signer available {signer_available}, codec {}",
                        requirement.program.instance().id().0, requirement.program.root(), codec.as_deref().unwrap_or("unspecified"))?;
                } else {
                    writeln!(
                        text,
                        "    Requirement: {}",
                        serde_json::to_string(requirement)?
                    )?;
                }
            }
            if let Some(weight) = branch.satisfaction_weight_upper_bound {
                writeln!(
                    text,
                    "    Selected satisfaction upper bound: {} wu",
                    weight.to_wu()
                )?;
            } else {
                writeln!(text, "    Satisfaction weight is unknown")?;
            }
        }
        writeln!(
            text,
            "  Capability declarations are not signatures, evaluator approval, or chain maturity."
        )?;
    }
    Ok(text)
}
