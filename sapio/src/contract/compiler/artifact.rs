//! Bind serialized policy claims to actual Taproot spending capabilities.

use super::*;

/// Replay the artifact's complete spending policy. Template covenants are
/// injected here, never inferred from untrusted descriptor membership claims.
pub(crate) fn validate_spending_policies(object: &Compiled) -> Result<(), String> {
    use miniscript::policy::Liftable;

    let has_claims = !object.ctv_to_tx.is_empty()
        || !object.program_policies.is_empty()
        || !object.committed_policy_guards.is_empty()
        || !object.covenant_requirements.predicates.is_empty()
        || !object.alternative_policies.is_empty();
    // Plain destinations do not advertise a contract spending policy.
    if !has_claims {
        return Ok(());
    }
    let descriptor = object
        .descriptor
        .as_ref()
        .ok_or_else(|| "spending policy requires a complete Taproot descriptor".to_string())?;
    if !matches!(
        descriptor,
        SupportedDescriptors::XOnly(Descriptor::Tr(_)) | SupportedDescriptors::Taproot(_)
    ) {
        return Err("spending policy requires a complete Taproot descriptor".into());
    }
    if !object
        .committed_policy_guards
        .keys()
        .eq(object.ctv_to_tx.keys())
    {
        return Err("committed branch records do not match the template catalog".into());
    }
    script::validate_record_sources(
        object
            .alternative_policies
            .iter()
            .chain(
                object
                    .ctv_to_tx
                    .values()
                    .flat_map(|template| &template.guards),
            )
            .chain(
                object
                    .committed_policy_guards
                    .values()
                    .flatten()
                    .flat_map(|record| {
                        std::iter::once(&record.action_guard).chain(&record.template_guards)
                    }),
            ),
    )
    .map_err(|error| error.to_string())?;
    let mut requirements = CovenantRequirements {
        lowering: object.covenant_requirements.lowering.clone(),
        predicates: BTreeSet::new(),
    };
    let mut branches = Vec::new();
    let mut bytes = 0;
    let mut budget = script::RecordedPolicyBudget::default();
    let mut append = |source: ScriptPolicy| -> Result<(), CompilationError> {
        let compiled = compile_branches(source, &mut budget)?;
        append_branches(&mut branches, &mut bytes, compiled)
    };
    for template in object.ctv_to_tx.values() {
        let covenant = ScriptPolicy::from(Emulatable(Ctv(template.hash())));
        let guards = &object.committed_policy_guards[&template.hash()];
        let recorded: BTreeSet<_> = guards
            .iter()
            .flat_map(|record| {
                source_alternatives(conjoin_source(
                    std::iter::once(&record.action_guard).chain(&record.template_guards),
                ))
            })
            .collect();
        let declared: BTreeSet<_> = source_alternatives(conjoin_source(template.guards.iter()))
            .into_iter()
            .collect();
        if recorded != declared {
            return Err("committed branch guards differ from the template's guards".into());
        }
        for record in guards {
            let guard = script::resolve_emulation(&record.action_guard, &mut requirements)
                .map_err(|error| error.to_string())?;
            let clause = script::resolve_emulation(
                &conjoin_source(
                    record
                        .template_guards
                        .iter()
                        .chain(std::iter::once(&covenant)),
                ),
                &mut requirements,
            )
            .map_err(|error| error.to_string())?;
            append(conjoin_source([&guard, &clause].into_iter()))
                .map_err(|error| error.to_string())?;
        }
    }
    for policy in &object.alternative_policies {
        let policy = script::resolve_emulation(policy, &mut requirements)
            .map_err(|error| error.to_string())?;
        append(policy).map_err(|error| error.to_string())?;
    }
    // Compilation can resolve predicates in subsequently discarded/false
    // alternatives. Those historical lowering records need not be executable;
    // every committed template above still gets a complete mandatory branch.
    if !requirements
        .predicates
        .is_subset(&object.covenant_requirements.predicates)
    {
        return Err("spending policy has unrecorded covenant predicates".into());
    }
    let mut expected = BTreeSet::new();
    let mut bare_keys = BTreeSet::new();
    let mut expected_programs = BTreeSet::new();
    for branch in branches {
        if let Some(policy) = branch.program_policy {
            expected_programs.insert(policy);
        }
        if let CompiledScript::Miniscript(script) = &branch.script {
            if let Some(key) = bare_key(script) {
                bare_keys.insert(key);
            }
        }
        expected.insert(branch.script.into_script());
    }
    if expected_programs.iter().collect::<BTreeSet<_>>()
        != object
            .program_policies
            .iter()
            .map(|record| &record.policy)
            .collect()
    {
        return Err("program policy records differ from the complete spending sources".into());
    }
    let mut input = bitcoin::psbt::Input::default();
    descriptor
        .update_psbt_input(&mut input)
        .map_err(|error| error.to_string())?;
    let internal = input
        .tap_internal_key
        .ok_or("missing Taproot internal key")?;
    if internal != unspendable_internal_key() && !bare_keys.contains(&internal) {
        return Err(
            "internal key is neither NUMS nor an independently sufficient policy branch".into(),
        );
    }
    let actual: BTreeSet<_> = input
        .tap_scripts
        .values()
        .map(|(script, _)| script.clone())
        .collect();
    for script in &actual {
        if !expected.contains(script) {
            return Err("descriptor contains an undeclared or weakened spending branch".into());
        }
        if Miniscript::<XOnlyPublicKey, Tap>::decode_consensus(script)
            .ok()
            .and_then(|script| script.lift().ok())
            .is_some_and(|policy| unconditional(&policy))
        {
            return Err("descriptor contains an unconditional spending branch".into());
        }
    }
    // An explicitly pinned bare-key branch may have been removed from the tree
    // after promotion. Every other complete branch must still be present.
    if bare_keys.contains(&internal) {
        expected.remove(
            &Clause::Key(internal)
                .compile::<Tap>()
                .map_err(|error| error.to_string())?
                .encode(),
        );
    }
    if !expected.is_subset(&actual) {
        return Err("descriptor is missing a committed or alternative spending branch".into());
    }
    Ok(())
}

fn unconditional(policy: &policy::semantic::Policy<XOnlyPublicKey>) -> bool {
    use policy::semantic::Policy;
    match policy {
        Policy::Trivial => true,
        Policy::Inscribe(_, inner) => unconditional(inner),
        Policy::Thresh(threshold) => {
            threshold
                .iter()
                .filter(|child| unconditional(child))
                .count()
                >= threshold.k()
        }
        _ => false,
    }
}
