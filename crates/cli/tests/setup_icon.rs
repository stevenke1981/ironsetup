//! Windows-only: SetupIconFile must land in the setup EXE as RT_GROUP_ICON and
//! the result must still verify and inspect (the footer derives the stub length).
#![cfg(windows)]
use std::{fs, os::windows::ffi::OsStrExt, path::Path, process::Command};
use windows_sys::Win32::{
    Foundation::FreeLibrary,
    System::LibraryLoader::{EnumResourceNamesW, LoadLibraryExW, LOAD_LIBRARY_AS_DATAFILE},
};

fn ico() -> Vec<u8> {
    let mut dib = vec![0u8; 40 + 16 * 16 * 4 + 16 * 4];
    dib[0] = 40;
    dib[4] = 16; // width
    dib[8] = 32; // height incl. mask
    dib[12] = 1; // planes
    dib[14] = 32; // bpp
    let mut b = vec![0, 0, 1, 0, 1, 0, 16, 16, 0, 0, 1, 0, 32, 0];
    b.extend_from_slice(&(dib.len() as u32).to_le_bytes());
    b.extend_from_slice(&22u32.to_le_bytes());
    b.extend_from_slice(&dib);
    b
}

unsafe extern "system" fn collect(
    _m: *mut core::ffi::c_void,
    _t: *const u16,
    name: *const u16,
    param: isize,
) -> i32 {
    let ids = &mut *(param as *mut Vec<usize>);
    ids.push(name as usize);
    1
}

const RT_GROUP_ICON: usize = 14;
const RT_MANIFEST: usize = 24;

fn resource_ids(exe: &Path, kind: usize) -> Vec<usize> {
    let wide: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut ids = Vec::new();
    unsafe {
        let module = LoadLibraryExW(
            wide.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_AS_DATAFILE,
        );
        assert!(!module.is_null(), "cannot load {}", exe.display());
        EnumResourceNamesW(
            module,
            kind as *const u16,
            Some(collect),
            &mut ids as *mut _ as isize,
        );
        FreeLibrary(module);
    }
    ids
}

fn compile(dir: &Path, extra: &str) -> std::process::Output {
    fs::create_dir_all(dir.join("payload")).unwrap();
    fs::write(dir.join("payload/app.txt"), "hello").unwrap();
    fs::write(dir.join("icon.ico"), ico()).unwrap();
    fs::write(
        dir.join("t.iss"),
        format!("[Setup]\nAppName=Icon\nAppVersion=1\nDefaultDirName={{localappdata}}\\IconTest\nPrivilegesRequired=lowest\nCompression=zip\n{extra}\n[Files]\nSource: \"payload\\app.txt\"; DestDir: \"{{app}}\"; Flags: ignoreversion\n"),
    )
    .unwrap();
    Command::new(env!("CARGO_BIN_EXE_ironsetup"))
        .args(["compile"])
        .arg(dir.join("t.iss"))
        .arg("--stub")
        .arg(env!("CARGO_BIN_EXE_ironsetup-stub"))
        .arg("--output")
        .arg(dir.join("setup.exe"))
        .output()
        .unwrap()
}

#[test]
fn setup_icon_is_embedded_and_exe_still_inspects() {
    let d = tempfile::tempdir().unwrap();
    let plain = tempfile::tempdir().unwrap();
    let out = compile(plain.path(), "");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let baseline = resource_ids(&plain.path().join("setup.exe"), RT_GROUP_ICON);
    // build.rs embeds the application manifest (asInvoker, ComCtl32 v6, ...).
    assert_eq!(
        resource_ids(&plain.path().join("setup.exe"), RT_MANIFEST),
        vec![1],
        "setup EXE must carry RT_MANIFEST #1"
    );

    let out = compile(
        d.path(),
        "SetupIconFile=icon.ico\nUninstallDisplayIcon={app}\\app.txt",
    );
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let exe = d.path().join("setup.exe");
    let ids = resource_ids(&exe, RT_GROUP_ICON);
    assert!(
        ids.contains(&1),
        "RT_GROUP_ICON id 1 missing: {ids:?} (baseline {baseline:?})"
    );
    // UpdateResource must add the icon without dropping the manifest.
    assert_eq!(
        resource_ids(&exe, RT_MANIFEST),
        vec![1],
        "SetupIconFile must keep RT_MANIFEST #1"
    );
    let inspect = Command::new(env!("CARGO_BIN_EXE_ironsetup"))
        .arg("inspect")
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    assert!(String::from_utf8_lossy(&inspect.stdout).contains("Icon"));
}

#[test]
fn invalid_setup_icon_fails_the_build() {
    let d = tempfile::tempdir().unwrap();
    let out = compile(d.path(), "SetupIconFile=payload\\app.txt");
    assert!(!out.status.success());
    assert!(!d.path().join("setup.exe").exists());
}
