use crate::InstallOptions;
use anyhow::{bail, Result};
use ironsetup_core::{evaluate_condition, Manifest};
use std::collections::BTreeSet;

pub fn default_selections(
    manifest: &Manifest,
    setup_type: Option<&str>,
) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
    let chosen = if let Some(name) = setup_type {
        Some(
            manifest
                .types
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| anyhow::anyhow!("Unknown setup type: {name}"))?,
        )
    } else {
        manifest.types.iter().find(|t| !t.custom)
    };
    let components = manifest
        .components
        .iter()
        .filter(|c| {
            c.flags.iter().any(|f| f == "fixed")
                || chosen.is_none()
                || c.types.is_empty()
                || chosen.is_some_and(|t| c.types.iter().any(|n| n.eq_ignore_ascii_case(&t.name)))
        })
        .map(|c| c.name.to_lowercase())
        .collect::<BTreeSet<_>>();
    let mut tasks = BTreeSet::new();
    for task in &manifest.tasks {
        if !task.conditions.tasks.is_empty() {
            bail!("Task-to-task conditions are unsupported");
        }
        if !task.flags.iter().any(|f| f == "unchecked")
            && evaluate_condition(&task.conditions.components, &components)?
        {
            tasks.insert(task.name.to_lowercase());
        }
    }
    Ok((components, tasks))
}

pub(crate) fn resolve_selections(
    manifest: &Manifest,
    options: &InstallOptions,
) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
    for item in &manifest.components {
        if !item.conditions.components.is_empty() || !item.conditions.tasks.is_empty() {
            bail!("Conditional component definitions are unsupported");
        }
        if item.flags.iter().any(|f| f != "fixed") {
            bail!("Unsupported component flags on {}", item.name);
        }
    }
    for item in &manifest.tasks {
        if item.flags.iter().any(|f| f != "unchecked") {
            bail!("Unsupported task flags on {}", item.name);
        }
    }
    let (mut components, mut tasks) = default_selections(manifest, options.setup_type.as_deref())?;
    if let Some(explicit) = &options.components {
        components = explicit.iter().map(|s| s.to_lowercase()).collect();
        for name in &components {
            if !manifest
                .components
                .iter()
                .any(|c| c.name.eq_ignore_ascii_case(name))
            {
                bail!("Unknown component: {name}");
            }
        }
        for fixed in manifest
            .components
            .iter()
            .filter(|c| c.flags.iter().any(|f| f == "fixed"))
        {
            components.insert(fixed.name.to_lowercase());
        }
        tasks.clear();
        for task in &manifest.tasks {
            if !task.flags.iter().any(|f| f == "unchecked")
                && evaluate_condition(&task.conditions.components, &components)?
            {
                tasks.insert(task.name.to_lowercase());
            }
        }
    }
    if let Some(explicit) = &options.tasks {
        tasks = explicit.iter().map(|s| s.to_lowercase()).collect();
        for name in &tasks {
            let task = manifest
                .tasks
                .iter()
                .find(|c| c.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| anyhow::anyhow!("Unknown task: {name}"))?;
            if !evaluate_condition(&task.conditions.components, &components)? {
                bail!("Task {name} requires a component that was not selected");
            }
        }
    }
    Ok((components, tasks))
}
