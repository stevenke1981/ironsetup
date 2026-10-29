use ironsetup_core::*;
use ironsetup_engine::{
    default_selections, install, recover, uninstall, verify_install, InstallOptions,
};
use std::collections::BTreeSet;
use std::fs;

fn package(entries: &[(&str, &[u8])]) -> Package {
    Package {
        manifest: Manifest {
            format_version: FORMAT_VERSION,
            app: AppInfo {
                id: "ironsetup.engine.fixture".into(),
                name: "測試應用程式".into(),
                version: "1.2.3".into(),
                publisher: "IronSetup Tests".into(),
                default_dir: "{userpf}/IronSetup Fixture".into(),
                default_group: "IronSetup Fixture".into(),
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

#[test]
fn fresh_install_verify_uninstall_preserves_unowned_files() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    let package = package(&[("bin/app.txt", b"application"), ("資料/說明.txt", b"hello")]);
    let result = install(&package, &InstallOptions::new(target.clone()), &mut |_| {}).unwrap();
    assert_eq!(result.installed_files.len(), 2);
    assert_eq!(fs::read(target.join("資料/說明.txt")).unwrap(), b"hello");
    assert!(verify_install(&target).unwrap().intact);
    fs::write(target.join("personal.txt"), b"keep me").unwrap();
    let result = uninstall(&target, &mut |_| {}).unwrap();
    assert_eq!(result.removed.len(), 2);
    assert!(!target.join("bin/app.txt").exists());
    assert_eq!(fs::read(target.join("personal.txt")).unwrap(), b"keep me");
    assert!(!target.join(STATE_DIR).exists());
}

#[test]
fn overwrites_are_backed_up_and_originals_restored() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("settings.txt"), b"original configuration").unwrap();
    let package = package(&[("settings.txt", b"installed defaults")]);
    install(&package, &InstallOptions::new(target.clone()), &mut |_| {}).unwrap();
    assert_eq!(
        fs::read(target.join("settings.txt")).unwrap(),
        b"installed defaults"
    );
    let report = uninstall(&target, &mut |_| {}).unwrap();
    assert_eq!(report.restored.len(), 1);
    assert_eq!(
        fs::read(target.join("settings.txt")).unwrap(),
        b"original configuration"
    );
    assert!(!target.join(STATE_DIR).exists());
}

#[test]
fn modified_overwritten_files_preserve_both_user_edit_and_original_backup() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("settings.txt"), b"before install").unwrap();
    install(
        &package(&[("settings.txt", b"installed")]),
        &InstallOptions::new(target.clone()),
        &mut |_| {},
    )
    .unwrap();
    fs::write(target.join("settings.txt"), b"user's new settings").unwrap();
    let verification = verify_install(&target).unwrap();
    assert!(!verification.intact);
    assert_eq!(verification.modified.len(), 1);
    let report = uninstall(&target, &mut |_| {}).unwrap();
    assert!(report.metadata_retained);
    assert_eq!(
        fs::read(target.join("settings.txt")).unwrap(),
        b"user's new settings"
    );
    assert_eq!(
        fs::read(target.join(STATE_DIR).join("backups/00000000.bin")).unwrap(),
        b"before install"
    );
}

#[test]
fn a_digest_failure_happens_before_any_directory_is_created() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    let mut package = package(&[("app.txt", b"correct")]);
    package.payloads[0] = b"corrupt".to_vec();
    assert!(install(&package, &InstallOptions::new(target.clone()), &mut |_| {}).is_err());
    assert!(!target.exists());
}

#[test]
fn failed_second_write_rolls_back_first_write_to_original_bytes() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("first.txt"), b"original").unwrap();
    let package = package(&[("first.txt", b"new value"), ("clash/second.txt", b"second")]);
    // A conflicting file appears after preflight and after the first replacement.
    let error = install(
        &package,
        &InstallOptions::new(target.clone()),
        &mut |event| {
            if event.stage == "install" && event.item == "clash/second.txt" {
                fs::write(target.join("clash"), b"not a directory").unwrap();
            }
        },
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("rolled back"), "{error:#}");
    assert_eq!(fs::read(target.join("first.txt")).unwrap(), b"original");
    assert_eq!(fs::read(target.join("clash")).unwrap(), b"not a directory");
    assert!(!target.join(STATE_DIR).exists());
}

#[test]
fn existing_optional_file_is_not_owned_or_removed() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("settings.txt"), b"personal").unwrap();
    let mut package = package(&[("settings.txt", b"default")]);
    package.manifest.files[0].flags = vec!["onlyifdoesntexist".into()];
    let result = install(&package, &InstallOptions::new(target.clone()), &mut |_| {}).unwrap();
    assert_eq!(result.skipped_files, vec!["settings.txt"]);
    uninstall(&target, &mut |_| {}).unwrap();
    assert_eq!(fs::read(target.join("settings.txt")).unwrap(), b"personal");
}

#[test]
fn interrupted_transaction_can_be_recovered_without_losing_originals() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("settings.txt"), b"original").unwrap();
    install(
        &package(&[("settings.txt", b"installed")]),
        &InstallOptions::new(target.clone()),
        &mut |_| {},
    )
    .unwrap();
    let journal_path = target.join(STATE_DIR).join("journal.json");
    let mut journal: serde_json::Value =
        serde_json::from_slice(&fs::read(&journal_path).unwrap()).unwrap();
    journal["status"] = "installing".into();
    fs::write(&journal_path, serde_json::to_vec(&journal).unwrap()).unwrap();
    assert!(uninstall(&target, &mut |_| {}).is_err());
    let report = recover(&target, &mut |_| {}).unwrap();
    assert_eq!(report.restored.len(), 1);
    assert_eq!(fs::read(target.join("settings.txt")).unwrap(), b"original");
    assert!(!target.join(STATE_DIR).exists());
}

#[test]
fn repeated_install_is_explicitly_refused() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    let package = package(&[("app.txt", b"one")]);
    install(&package, &InstallOptions::new(target.clone()), &mut |_| {}).unwrap();
    let error = install(&package, &InstallOptions::new(target.clone()), &mut |_| {}).unwrap_err();
    assert!(error.to_string().contains("already exists"));
    assert!(verify_install(&target).unwrap().intact);
}

#[test]
fn explicit_components_include_fixed_and_validate_task_dependencies() {
    let mut package = package(&[("core.txt", b"core"), ("extra.txt", b"extra")]);
    package.manifest.types = vec![
        SetupType {
            name: "minimal".into(),
            description: "Minimal".into(),
            custom: false,
        },
        SetupType {
            name: "full".into(),
            description: "Full".into(),
            custom: false,
        },
    ];
    package.manifest.components = vec![
        Choice {
            name: "core".into(),
            description: "Core".into(),
            types: vec!["minimal".into(), "full".into()],
            flags: vec!["fixed".into()],
            conditions: Conditions::default(),
        },
        Choice {
            name: "extra".into(),
            description: "Extra".into(),
            types: vec!["full".into()],
            flags: Vec::new(),
            conditions: Conditions::default(),
        },
    ];
    package.manifest.files[0].conditions.components = "core".into();
    package.manifest.files[1].conditions.components = "extra".into();
    package.manifest.tasks = vec![Choice {
        name: "sample".into(),
        description: "Sample".into(),
        types: Vec::new(),
        flags: Vec::new(),
        conditions: Conditions {
            components: "extra".into(),
            tasks: String::new(),
        },
    }];
    let (components, tasks) = default_selections(&package.manifest, None).unwrap();
    assert_eq!(components, BTreeSet::from(["core".into()]));
    assert!(tasks.is_empty());
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    let mut options = InstallOptions::new(target.clone());
    options.components = Some(BTreeSet::new());
    options.tasks = Some(BTreeSet::from(["sample".into()]));
    assert!(install(&package, &options, &mut |_| {}).is_err());
    assert!(!target.exists());
    options.tasks = Some(BTreeSet::new());
    let result = install(&package, &options, &mut |_| {}).unwrap();
    assert!(result.selected_components.contains("core"));
    assert!(target.join("core.txt").exists());
    assert!(!target.join("extra.txt").exists());
}

#[test]
fn traversal_and_case_collisions_fail_before_mutation() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    for bad in [
        "../outside",
        "C:/outside",
        "dir/../outside",
        "name:stream",
        ".ironsetup/journal.json",
    ] {
        assert!(install(
            &package(&[(bad, b"bad")]),
            &InstallOptions::new(target.clone()),
            &mut |_| {}
        )
        .is_err());
        assert!(!target.exists());
    }
    assert!(install(
        &package(&[("Test.txt", b"first"), ("test.txt", b"second")]),
        &InstallOptions::new(target.clone()),
        &mut |_| {}
    )
    .is_err());
}

#[test]
fn tampered_uninstall_path_is_rejected_without_touching_other_files() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    let outside = temporary.path().join("outside.txt");
    fs::write(&outside, b"important").unwrap();
    install(
        &package(&[("app.txt", b"data")]),
        &InstallOptions::new(target.clone()),
        &mut |_| {},
    )
    .unwrap();
    let path = target.join(STATE_DIR).join("journal.json");
    let mut journal: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    journal["files"][0]["location"]["path"] = "../outside.txt".into();
    fs::write(&path, serde_json::to_vec(&journal).unwrap()).unwrap();
    assert!(uninstall(&target, &mut |_| {}).is_err());
    assert_eq!(fs::read(outside).unwrap(), b"important");
    assert!(target.join("app.txt").exists());
}

#[test]
fn uninstallable_false_still_has_safe_installation_metadata() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    let mut package = package(&[("app.txt", b"data")]);
    package.manifest.app.uninstallable = false;
    install(&package, &InstallOptions::new(target.clone()), &mut |_| {}).unwrap();
    assert!(verify_install(&target).unwrap().intact);
    assert!(uninstall(&target, &mut |_| {})
        .unwrap_err()
        .to_string()
        .contains("Uninstallable=no"));
    assert!(target.join("app.txt").exists());
}

#[test]
fn keep_flag_retains_installed_file() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    let mut package = package(&[("persistent.txt", b"keep")]);
    package.manifest.files[0]
        .flags
        .push("uninsneveruninstall".into());
    install(&package, &InstallOptions::new(target.clone()), &mut |_| {}).unwrap();
    let result = uninstall(&target, &mut |_| {}).unwrap();
    assert_eq!(result.preserved.len(), 1);
    assert_eq!(fs::read(target.join("persistent.txt")).unwrap(), b"keep");
}

#[test]
fn an_active_transaction_blocks_a_second_operation() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    let mut was_blocked = false;
    install(
        &package(&[("app.txt", b"content")]),
        &InstallOptions::new(target.clone()),
        &mut |event| {
            if event.stage == "install" {
                was_blocked = verify_install(&target).is_err();
            }
        },
    )
    .unwrap();
    assert!(
        was_blocked,
        "concurrent operation must not obtain the metadata lock"
    );
    assert!(verify_install(&target).unwrap().intact);
}

#[cfg(unix)]
#[test]
fn symlink_destinations_do_not_escape_installation_root() {
    use std::os::unix::fs::symlink;
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    let outside = temporary.path().join("outside");
    fs::create_dir(&target).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("important.txt"), b"original").unwrap();
    symlink(&outside, target.join("link")).unwrap();
    assert!(install(
        &package(&[("link/important.txt", b"overwrite")]),
        &InstallOptions::new(target.clone()),
        &mut |_| {}
    )
    .is_err());
    assert_eq!(
        fs::read(outside.join("important.txt")).unwrap(),
        b"original"
    );
    assert!(!target.join(STATE_DIR).exists());
}

#[cfg(not(windows))]
#[test]
fn windows_only_actions_fail_before_creating_directories() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    let mut package = package(&[("app.txt", b"data")]);
    package.manifest.registry.push(RegistryEntry {
        root: "HKCU".into(),
        subkey: "Software\\IronSetupFixture".into(),
        value_type: "string".into(),
        value_name: "Test".into(),
        value_data: "one".into(),
        flags: vec!["uninsdeletevalue".into()],
        conditions: Conditions::default(),
    });
    assert!(
        install(&package, &InstallOptions::new(target.clone()), &mut |_| {})
            .unwrap_err()
            .to_string()
            .contains("Windows")
    );
    assert!(!target.exists());
}

#[cfg(unix)]
#[test]
fn run_parameters_are_not_evaluated_as_a_shell_command() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    let mut package = package(&[(
        "collect.sh",
        b"#!/bin/sh\nprintf '%s\\n' \"$@\" > arguments.txt\nexit 7\n",
    )]);
    package.manifest.files[0].executable = true;
    package.manifest.run.push(RunEntry {
        filename: "{app}/collect.sh".into(),
        parameters: "hello \"two words\" \";\" \"echo BAD\"".into(),
        working_dir: String::new(),
        description: String::new(),
        flags: Vec::new(),
        conditions: Conditions::default(),
    });
    let report = install(&package, &InstallOptions::new(target.clone()), &mut |_| {}).unwrap();
    assert_eq!(
        fs::read_to_string(target.join("arguments.txt")).unwrap(),
        "hello\ntwo words\n;\necho BAD\n"
    );
    assert!(report.warnings.iter().any(|w| w.contains("exited with 7")));
    assert!(verify_install(&target).unwrap().intact);
    uninstall(&target, &mut |_| {}).unwrap();
    assert!(target.join("arguments.txt").exists());
}
