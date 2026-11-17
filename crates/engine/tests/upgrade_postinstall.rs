//! In-place upgrade over a same-AppId installation and deferred postinstall
//! [Run] entries.
use ironsetup_core::*;
use ironsetup_engine::{install, run_postinstall, uninstall, verify_install, InstallOptions};
use std::collections::BTreeSet;
use std::fs;

fn package(version: &str, entries: &[(&str, &[u8])]) -> Package {
    Package {
        manifest: Manifest {
            format_version: FORMAT_VERSION,
            app: AppInfo {
                id: "ironsetup.engine.upgrade".into(),
                name: "Upgrade Fixture".into(),
                version: version.into(),
                publisher: "IronSetup Tests".into(),
                default_dir: "{userpf}/IronSetup Upgrade".into(),
                default_group: "IronSetup Upgrade".into(),
                privileges: "lowest".into(),
                uninstallable: true,
                compression: "zip".into(),
                architectures_allowed: String::new(),
            },
            files: entries
                .iter()
                .map(|(path, data)| ManifestFile {
                    path: (*path).into(),
                    size: data.len() as u64,
                    sha256: sha256(data),
                    executable: false,
                    flags: vec!["ignoreversion".into()],
                    conditions: Conditions::default(),
                })
                .collect(),
            dirs: Vec::new(),
            components: Vec::new(),
            tasks: Vec::new(),
            types: Vec::new(),
            icons: Vec::new(),
            registry: Vec::new(),
            run: Vec::new(),
            license_text: None,
            info_before: None,
            info_after: None,
        },
        payloads: entries.iter().map(|(_, data)| data.to_vec()).collect(),
    }
}

fn snapshot(target: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    let mut stack = vec![target.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push((path.display().to_string(), fs::read(&path).unwrap()));
            }
        }
    }
    files.sort();
    files
}

#[test]
fn same_app_id_upgrades_in_place_and_uninstalls_cleanly() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    let v1 = package(
        "1.0",
        &[("app.txt", b"version one"), ("old-only.txt", b"old")],
    );
    install(&v1, &InstallOptions::new(target.clone()), &mut |_| {}).unwrap();
    fs::write(target.join("user-data.txt"), b"mine").unwrap();
    let v2 = package(
        "2.0",
        &[("app.txt", b"version two"), ("new-only.txt", b"new")],
    );
    let report = install(&v2, &InstallOptions::new(target.clone()), &mut |_| {}).unwrap();
    assert_eq!(report.upgraded_from.as_deref(), Some("1.0"));
    assert_eq!(fs::read(target.join("app.txt")).unwrap(), b"version two");
    assert_eq!(fs::read(target.join("new-only.txt")).unwrap(), b"new");
    assert!(!target.join("old-only.txt").exists());
    assert_eq!(fs::read(target.join("user-data.txt")).unwrap(), b"mine");
    let verification = verify_install(&target).unwrap();
    assert!(verification.intact);
    assert_eq!(verification.verified.len(), 2);
    // The new journal owns only the new version and no stale backup of v1.
    let removed = uninstall(&target, &mut |_| {}).unwrap();
    assert!(removed.restored.is_empty());
    assert!(!removed.metadata_retained);
    assert!(!target.join("app.txt").exists());
    assert!(!target.join(STATE_DIR).exists());
    assert_eq!(fs::read(target.join("user-data.txt")).unwrap(), b"mine");
}

#[test]
fn upgrade_preserves_original_pre_install_files_for_final_uninstall() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("settings.txt"), b"before any install").unwrap();
    install(
        &package("1.0", &[("settings.txt", b"v1 defaults")]),
        &InstallOptions::new(target.clone()),
        &mut |_| {},
    )
    .unwrap();
    install(
        &package("2.0", &[("settings.txt", b"v2 defaults")]),
        &InstallOptions::new(target.clone()),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(
        fs::read(target.join("settings.txt")).unwrap(),
        b"v2 defaults"
    );
    let report = uninstall(&target, &mut |_| {}).unwrap();
    assert_eq!(report.restored.len(), 1);
    assert_eq!(
        fs::read(target.join("settings.txt")).unwrap(),
        b"before any install"
    );
    assert!(!target.join(STATE_DIR).exists());
}

#[test]
fn only_if_doesnt_exist_files_are_planned_after_the_old_version_is_removed() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    let mut v1 = package("1.0", &[("config.ini", b"default")]);
    v1.manifest.files[0].flags = vec!["onlyifdoesntexist".into()];
    install(&v1, &InstallOptions::new(target.clone()), &mut |_| {}).unwrap();
    let mut v2 = package("2.0", &[("config.ini", b"default v2")]);
    v2.manifest.files[0].flags = vec!["onlyifdoesntexist".into()];
    let report = install(&v2, &InstallOptions::new(target.clone()), &mut |_| {}).unwrap();
    assert_eq!(report.installed_files, vec!["config.ini".to_string()]);
    assert_eq!(fs::read(target.join("config.ini")).unwrap(), b"default v2");
}

#[test]
fn different_app_id_is_refused_without_changes() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    install(
        &package("1.0", &[("app.txt", b"one")]),
        &InstallOptions::new(target.clone()),
        &mut |_| {},
    )
    .unwrap();
    let before = snapshot(&target);
    let mut other = package("9.0", &[("app.txt", b"other")]);
    other.manifest.app.id = "someone.else".into();
    let error = install(&other, &InstallOptions::new(target.clone()), &mut |_| {}).unwrap_err();
    assert!(
        format!("{error:#}").contains("different application"),
        "{error:#}"
    );
    assert_eq!(snapshot(&target), before);
    assert!(verify_install(&target).unwrap().intact);
}

#[test]
fn interrupted_transaction_is_refused_without_changes() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    install(
        &package("1.0", &[("app.txt", b"one")]),
        &InstallOptions::new(target.clone()),
        &mut |_| {},
    )
    .unwrap();
    let journal_path = target.join(STATE_DIR).join("journal.json");
    let mut journal: serde_json::Value =
        serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
    journal["status"] = "installing".into();
    fs::write(&journal_path, serde_json::to_vec(&journal).unwrap()).unwrap();
    let before = snapshot(&target);
    let error = install(
        &package("2.0", &[("app.txt", b"two")]),
        &InstallOptions::new(target.clone()),
        &mut |_| {},
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("recover"), "{error:#}");
    assert_eq!(snapshot(&target), before);
}

#[test]
fn modified_file_with_backed_up_original_refuses_upgrade_without_changes() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("settings.txt"), b"original").unwrap();
    install(
        &package("1.0", &[("settings.txt", b"installed")]),
        &InstallOptions::new(target.clone()),
        &mut |_| {},
    )
    .unwrap();
    fs::write(target.join("settings.txt"), b"user edit").unwrap();
    let before = snapshot(&target);
    let error = install(
        &package("2.0", &[("settings.txt", b"v2")]),
        &InstallOptions::new(target.clone()),
        &mut |_| {},
    )
    .unwrap_err();
    assert!(
        format!("{error:#}").contains("Nothing was changed"),
        "{error:#}"
    );
    assert_eq!(snapshot(&target), before);
}

#[cfg(windows)]
#[test]
fn file_held_by_a_running_program_refuses_upgrade_without_changes() {
    use std::os::windows::fs::OpenOptionsExt;
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    install(
        &package(
            "1.0",
            &[("app.exe", b"pretend program"), ("data.txt", b"data")],
        ),
        &InstallOptions::new(target.clone()),
        &mut |_| {},
    )
    .unwrap();
    let before = snapshot(&target);
    // Like a running executable image: readers allowed, no writer sharing.
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(1 /* FILE_SHARE_READ */)
        .open(target.join("app.exe"))
        .unwrap();
    let error = install(
        &package("2.0", &[("app.exe", b"new program")]),
        &InstallOptions::new(target.clone()),
        &mut |_| {},
    )
    .unwrap_err();
    let message = format!("{error:#}");
    assert!(message.contains("Close Upgrade Fixture"), "{message}");
    assert!(message.contains("Nothing was changed"), "{message}");
    drop(held);
    assert_eq!(snapshot(&target), before);
    assert!(verify_install(&target).unwrap().intact);
}

fn probe_package() -> Package {
    #[cfg(windows)]
    let (name, bytes) = (
        "probe.exe",
        fs::read(
            std::path::Path::new(&std::env::var("SystemRoot").unwrap())
                .join("System32")
                .join("whoami.exe"),
        )
        .unwrap(),
    );
    #[cfg(not(windows))]
    let (name, bytes) = ("probe.sh", b"#!/bin/sh\necho ran > ran.txt\n".to_vec());
    let mut package = package("1.0", &[(name, &bytes)]);
    package.manifest.files[0].executable = true;
    package.manifest.run = vec![
        RunEntry {
            filename: format!("{{app}}/{name}"),
            parameters: String::new(),
            working_dir: String::new(),
            description: "Launch probe".into(),
            flags: vec!["postinstall".into()],
            conditions: Conditions::default(),
        },
        RunEntry {
            filename: "{app}/missing.exe".into(),
            parameters: String::new(),
            working_dir: String::new(),
            description: "Launch missing".into(),
            flags: vec!["postinstall".into(), "skipifsilent".into()],
            conditions: Conditions::default(),
        },
    ];
    package
}

#[test]
fn interactive_install_defers_postinstall_entries_to_the_host() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    let package = probe_package();
    let mut options = InstallOptions::new(target.clone());
    options.defer_postinstall = true;
    let report = install(&package, &options, &mut |_| {}).unwrap();
    assert_eq!(report.postinstall, vec![0, 1]);
    assert!(
        !report
            .warnings
            .iter()
            .any(|w| w.contains("run") && w.contains("probe") || w.contains("missing.exe")),
        "{:?}",
        report.warnings
    );
    #[cfg(not(windows))]
    assert!(!target.join("ran.txt").exists());
    let none = BTreeSet::new();
    let ran = run_postinstall(&package, &target, &none, &none, &[0, 1]).unwrap();
    assert_eq!(ran.started.len(), 1);
    assert!(
        ran.failures.iter().any(|f| f.contains("missing.exe")),
        "{:?}",
        ran.failures
    );
    #[cfg(not(windows))]
    assert!(target.join("ran.txt").exists());
    // Only enabled postinstall entries of this installation can be requested.
    assert!(run_postinstall(&package, &target, &none, &none, &[2]).is_err());
    assert!(run_postinstall(&package, &target, &none, &none, &[0, 0]).is_err());
    let mut plain = package.clone();
    plain.manifest.run[0].flags.clear();
    assert!(run_postinstall(&plain, &target, &none, &none, &[0]).is_err());
    let mut other = package.clone();
    other.manifest.app.id = "someone.else".into();
    assert!(run_postinstall(&other, &target, &none, &none, &[0]).is_err());
}

#[test]
fn silent_install_keeps_running_postinstall_entries_unless_skipifsilent() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    let package = probe_package();
    let mut options = InstallOptions::new(target.clone());
    options.silent = true;
    options.defer_postinstall = true; // ignored in silent mode
    let report = install(&package, &options, &mut |_| {}).unwrap();
    assert!(report.postinstall.is_empty());
    assert!(
        !report
            .warnings
            .iter()
            .any(|w| w.contains("run") && w.contains("probe") || w.contains("missing.exe")),
        "{:?}",
        report.warnings
    );
    #[cfg(not(windows))]
    assert!(target.join("ran.txt").exists());
}

#[test]
fn postinstall_requires_description_and_non_admin_package() {
    let temporary = tempfile::tempdir().unwrap();
    let mut package = probe_package();
    package.manifest.run[0].description = " ".into();
    assert!(install(
        &package,
        &InstallOptions::new(temporary.path().join("a")),
        &mut |_| {}
    )
    .unwrap_err()
    .to_string()
    .contains("Description"));
    let mut package = probe_package();
    package.manifest.app.privileges = "admin".into();
    assert!(validate_package(&package)
        .unwrap_err()
        .to_string()
        .contains("elevated"));
    assert!(!temporary.path().join("a").exists());
}
