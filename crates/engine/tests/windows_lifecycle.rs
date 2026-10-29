#![cfg(windows)]
//! Runs on native Windows or Wine. Each invocation uses an isolated HKCU test
//! key and unique per-user Start Menu group; it never touches machine settings.
use ironsetup_core::*;
use ironsetup_engine::{install, uninstall, verify_install, InstallOptions};
use std::fs;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::*;

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn read_string(subkey: &str, name: &str) -> Option<String> {
    unsafe {
        let mut key = std::ptr::null_mut();
        let status = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            wide(subkey).as_ptr(),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        );
        if status == ERROR_FILE_NOT_FOUND {
            return None;
        }
        assert_eq!(status, ERROR_SUCCESS, "RegOpenKeyExW");
        let mut kind = 0;
        let mut size = 0;
        let status = RegQueryValueExW(
            key,
            wide(name).as_ptr(),
            std::ptr::null(),
            &mut kind,
            std::ptr::null_mut(),
            &mut size,
        );
        if status == ERROR_FILE_NOT_FOUND {
            RegCloseKey(key);
            return None;
        }
        assert_eq!(status, ERROR_SUCCESS, "RegQueryValueExW length");
        let mut data = vec![0u16; size.div_ceil(2) as usize];
        let status = RegQueryValueExW(
            key,
            wide(name).as_ptr(),
            std::ptr::null(),
            &mut kind,
            data.as_mut_ptr().cast(),
            &mut size,
        );
        RegCloseKey(key);
        assert_eq!(status, ERROR_SUCCESS, "RegQueryValueExW data");
        assert_eq!(kind, REG_SZ);
        while data.last() == Some(&0) {
            data.pop();
        }
        Some(String::from_utf16(&data).unwrap())
    }
}
fn set_original(subkey: &str) {
    unsafe {
        let mut key = std::ptr::null_mut();
        let mut disposition = 0;
        assert_eq!(
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                wide(subkey).as_ptr(),
                0,
                std::ptr::null(),
                0,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut key,
                &mut disposition
            ),
            ERROR_SUCCESS
        );
        let value = wide("original");
        assert_eq!(
            RegSetValueExW(
                key,
                wide("Value").as_ptr(),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32
            ),
            ERROR_SUCCESS
        );
        RegCloseKey(key);
    }
}
fn programs_folder() -> PathBuf {
    unsafe {
        use windows::Win32::UI::Shell::*;
        let value = SHGetKnownFolderPath(&FOLDERID_Programs, KF_FLAG_DEFAULT, None).unwrap();
        let path = PathBuf::from(value.to_string().unwrap());
        windows::Win32::System::Com::CoTaskMemFree(Some(value.0.cast()));
        path
    }
}
struct Cleanup {
    target: PathBuf,
    key: String,
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        if self.target.join(STATE_DIR).exists() {
            let _ = uninstall(&self.target, &mut |_| {});
        }
        unsafe {
            RegDeleteKeyExW(HKEY_CURRENT_USER, wide(&self.key).as_ptr(), 0, 0);
        }
    }
}
fn package(target: &Path, unique: &str) -> Package {
    let data = b"Native Windows fixture".to_vec();
    Package {
        manifest: Manifest {
            format_version: FORMAT_VERSION,
            app: AppInfo {
                id: format!("ironsetup.test.{unique}"),
                name: "IronSetup Windows Fixture".into(),
                version: "1.0.0".into(),
                publisher: "IronSetup Tests".into(),
                default_dir: target.to_string_lossy().into_owned(),
                default_group: format!("IronSetup QA {unique}"),
                privileges: "lowest".into(),
                uninstallable: true,
                compression: "zip".into(),
                architectures_allowed: "x64compatible".into(),
            },
            files: vec![ManifestFile {
                path: "app.txt".into(),
                size: data.len() as u64,
                sha256: sha256(&data),
                executable: false,
                flags: vec!["ignoreversion".into()],
                conditions: Conditions::default(),
            }],
            dirs: Vec::new(),
            components: Vec::new(),
            tasks: Vec::new(),
            types: Vec::new(),
            icons: vec![IconEntry {
                name: "{group}\\Fixture".into(),
                filename: "{app}\\app.txt".into(),
                parameters: String::new(),
                working_dir: "{app}".into(),
                comment: "IronSetup native API fixture".into(),
                icon_filename: String::new(),
                conditions: Conditions::default(),
            }],
            registry: vec![RegistryEntry {
                root: "HKCU".into(),
                subkey: format!("Software\\IronSetupTests\\Lifecycle_{unique}"),
                value_type: "string".into(),
                value_name: "Value".into(),
                value_data: "installed".into(),
                flags: vec!["uninsdeletevalue".into()],
                conditions: Conditions::default(),
            }],
            run: Vec::new(),
            license_text: None,
            info_before: None,
            info_after: None,
        },
        payloads: vec![data],
    }
}

#[test]
fn native_registry_shortcut_and_arp_install_uninstall() {
    let temporary = tempfile::tempdir().unwrap();
    let target = temporary.path().join("app");
    let unique = format!(
        "{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let package = package(&target, &unique);
    let subkey = package.manifest.registry[0].subkey.clone();
    let arp = format!(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\IronSetup_{}",
        &sha256(package.manifest.app.id.as_bytes())[..32]
    );
    let link = programs_folder()
        .join(&package.manifest.app.default_group)
        .join("Fixture.lnk");
    let _cleanup = Cleanup {
        target: target.clone(),
        key: subkey.clone(),
    };
    set_original(&subkey);
    let mut options = InstallOptions::new(target.clone());
    options.uninstaller = Some(fs::read(std::env::current_exe().unwrap()).unwrap());
    install(&package, &options, &mut |_| {}).unwrap();
    assert_eq!(read_string(&subkey, "Value").as_deref(), Some("installed"));
    assert_eq!(
        read_string(&arp, "DisplayName").as_deref(),
        Some("IronSetup Windows Fixture")
    );
    let uninstall_command = read_string(&arp, "UninstallString").unwrap();
    assert!(uninstall_command.contains("unins000.exe"));
    assert!(!uninstall_command.contains("\\\\?\\"));
    assert!(target.join("unins000.exe").is_file());
    let bytes = fs::read(&link).unwrap();
    assert_eq!(&bytes[..4], &[0x4c, 0, 0, 0], "Shell Link header");
    assert!(verify_install(&target).unwrap().intact);
    let report = uninstall(&target, &mut |_| {}).unwrap();
    assert!(!report.metadata_retained);
    assert_eq!(read_string(&subkey, "Value").as_deref(), Some("original"));
    assert!(read_string(&arp, "DisplayName").is_none());
    assert!(!link.exists());
    assert!(!link.parent().unwrap().exists());
    assert!(!target.exists());
}
