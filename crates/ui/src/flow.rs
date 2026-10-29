use anyhow::{bail, Result};
use ironsetup_core::{evaluate_condition, Manifest};
use std::collections::BTreeSet;

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    Welcome,
    Information,
    License,
    Destination,
    Features,
    Ready,
    Progress,
    Complete,
}

#[cfg(any(windows, test))]
pub(crate) fn pages(manifest: &Manifest) -> Vec<Page> {
    let mut pages = vec![Page::Welcome];
    if manifest.info_before.is_some() {
        pages.push(Page::Information);
    }
    if manifest.license_text.is_some() {
        pages.push(Page::License);
    }
    pages.push(Page::Destination);
    if !manifest.components.is_empty() || !manifest.tasks.is_empty() {
        pages.push(Page::Features);
    }
    pages.extend([Page::Ready, Page::Progress, Page::Complete]);
    pages
}

pub(crate) fn default_selections(
    manifest: &Manifest,
    setup_type: Option<&str>,
) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
    let selected_type = if let Some(name) = setup_type {
        Some(
            manifest
                .types
                .iter()
                .find(|ty| ty.name == name)
                .ok_or_else(|| anyhow::anyhow!("Unknown setup type: {name}"))?,
        )
    } else {
        manifest.types.iter().find(|ty| !ty.custom)
    };
    let components = manifest
        .components
        .iter()
        .filter(|choice| {
            choice.flags.iter().any(|flag| flag == "fixed")
                || selected_type.is_none()
                || choice.types.is_empty()
                || selected_type.is_some_and(|ty| choice.types.contains(&ty.name))
        })
        .map(|choice| choice.name.clone())
        .collect::<BTreeSet<_>>();
    let mut tasks = BTreeSet::new();
    for task in &manifest.tasks {
        if !task.conditions.tasks.is_empty() {
            bail!("Task defaults cannot depend on other tasks: {}", task.name);
        }
        if !task.flags.iter().any(|flag| flag == "unchecked")
            && evaluate_condition(&task.conditions.components, &components)?
        {
            tasks.insert(task.name.clone());
        }
    }
    Ok((components, tasks))
}

#[cfg(any(windows, test))]
pub(crate) fn crlf(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironsetup_core::{AppInfo, Choice, Conditions, SetupType, FORMAT_VERSION};

    fn manifest() -> Manifest {
        Manifest {
            format_version: FORMAT_VERSION,
            app: AppInfo {
                id: "test".into(),
                name: "Test".into(),
                version: "1".into(),
                publisher: String::new(),
                default_dir: "{localappdata}\\Test".into(),
                default_group: "Test".into(),
                privileges: "lowest".into(),
                uninstallable: true,
                compression: "zip".into(),
                architectures_allowed: String::new(),
            },
            files: vec![],
            dirs: vec![],
            components: vec![],
            tasks: vec![],
            types: vec![],
            icons: vec![],
            registry: vec![],
            run: vec![],
            license_text: None,
            info_before: None,
            info_after: None,
        }
    }

    #[test]
    fn license_and_information_cannot_be_skipped_by_default_route() {
        let mut m = manifest();
        m.license_text = Some("Terms".into());
        m.info_before = Some("Before installing".into());
        let route = pages(&m);
        assert!(
            route.iter().position(|p| *p == Page::License).unwrap()
                < route.iter().position(|p| *p == Page::Ready).unwrap()
        );
        assert_eq!(route[1], Page::Information);
        assert_eq!(*route.last().unwrap(), Page::Complete);
    }

    #[test]
    fn types_fixed_components_and_conditional_task_defaults() {
        let mut m = manifest();
        m.types = vec![SetupType {
            name: "minimal".into(),
            description: "Minimal".into(),
            custom: false,
        }];
        let choice = |name: &str, types: Vec<&str>, flags: Vec<&str>| Choice {
            name: name.into(),
            description: name.into(),
            types: types.into_iter().map(str::to_string).collect(),
            flags: flags.into_iter().map(str::to_string).collect(),
            conditions: Conditions::default(),
        };
        m.components = vec![
            choice("main", vec!["full"], vec!["fixed"]),
            choice("docs", vec!["full"], vec![]),
            choice("base", vec!["minimal"], vec![]),
        ];
        m.tasks = vec![
            choice("shortcut", vec![], vec![]),
            choice("optional", vec![], vec!["unchecked"]),
            choice("docs_task", vec![], vec![]),
        ];
        m.tasks[2].conditions.components = "docs".into();
        let (components, tasks) = default_selections(&m, None).unwrap();
        assert_eq!(components, BTreeSet::from(["main".into(), "base".into()]));
        assert_eq!(tasks, BTreeSet::from(["shortcut".into()]));
        assert!(default_selections(&m, Some("missing")).is_err());
    }

    #[test]
    fn unicode_editor_line_endings_are_normalized_once() {
        let original = "[Setup]\r\nAppName=繁體中文🦀\nAppVersion=1\r";
        let text = crlf(original);
        assert_eq!(text, "[Setup]\r\nAppName=繁體中文🦀\r\nAppVersion=1\r\n");
        assert_eq!(crlf(&text), text);
    }
}
