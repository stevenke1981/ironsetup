//! Independent review regressions against engine commit 2642f32.
//! No production code in this review worktree is modified.
#![cfg(unix)]

use ironsetup_core::*;
use ironsetup_engine::{install, uninstall, InstallOptions};
use std::fs;

fn package(data: &[u8]) -> Package {
    Package {
        manifest: Manifest {
            format_version: FORMAT_VERSION,
            app: AppInfo {
                id: "ironsetup.independent.review".into(),
                name: "Review Fixture".into(),
                version: "1.0".into(),
                publisher: String::new(),
                default_dir: "{userpf}/Review Fixture".into(),
                default_group: "Review Fixture".into(),
                privileges: "lowest".into(),
                uninstallable: true,
                compression: "zip".into(),
                architectures_allowed: String::new(),
            },
            files: vec![ManifestFile {
                path: "settings.txt".into(),
                size: data.len() as u64,
                sha256: sha256(data),
                executable: false,
                flags: vec!["ignoreversion".into()],
                conditions: Conditions::default(),
            }],
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
        },
        payloads: vec![data.to_vec()],
    }
}

#[cfg(unix)]
#[test]
fn journal_cannot_redirect_uninstall_to_unrelated_nonshortcut_file() {
    // XDG_DATA_HOME is used for its actual purpose: a disposable user-data root.
    // This test is run alone with --test-threads=1 and restores the original value.
    struct Restore(std::option::Option<std::ffi::OsString>);
    impl Drop for Restore {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var("XDG_DATA_HOME", value),
                None => std::env::remove_var("XDG_DATA_HOME"),
            }
        }
    }
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    let userdata = temporary.path().join("userdata");
    let outside = userdata.join("applications/private-notes.txt");
    fs::create_dir_all(outside.parent().unwrap()).unwrap();
    fs::write(&outside, b"important unrelated user notes").unwrap();
    let _restore = Restore(std::env::var_os("XDG_DATA_HOME"));
    std::env::set_var("XDG_DATA_HOME", &userdata);
    install(
        &package(b"application settings"),
        &InstallOptions::new(target.clone()),
        &mut |_| {},
    )
    .unwrap();
    let path = target.join(STATE_DIR).join("journal.json");
    let mut journal: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    journal["files"][0]["location"]["root"] = "programs".into();
    journal["files"][0]["location"]["path"] = "private-notes.txt".into();
    journal["files"][0]["installed_sha256"] = sha256(b"important unrelated user notes").into();
    fs::write(&path, serde_json::to_vec(&journal).unwrap()).unwrap();
    let outcome = uninstall(&target, &mut |_| {});
    assert!(
        outside.exists(),
        "uninstall deleted an unrelated non-shortcut file outside the app: {outcome:?}"
    );
    assert_eq!(
        fs::read(&outside).unwrap(),
        b"important unrelated user notes"
    );
}

#[cfg(unix)]
#[test]
fn equal_content_original_file_permissions_are_restored() {
    use std::os::unix::fs::PermissionsExt;
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    fs::create_dir(&target).unwrap();
    let settings = target.join("settings.txt");
    fs::write(&settings, b"same content").unwrap();
    fs::set_permissions(&settings, fs::Permissions::from_mode(0o600)).unwrap();
    install(
        &package(b"same content"),
        &InstallOptions::new(target.clone()),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(
        fs::metadata(&settings).unwrap().permissions().mode() & 0o777,
        0o644
    );
    uninstall(&target, &mut |_| {}).unwrap();
    assert_eq!(fs::metadata(&settings).unwrap().permissions().mode() & 0o777, 0o600,
        "uninstall must restore pre-install access permissions even when original and installed bytes have equal digests");
}
