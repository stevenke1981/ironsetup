//! Explicit GUI smoke test. This drives only windows owned by this test process.
//! Run on Windows, or under Wine with an X display:
//!   cargo test -p ironsetup-ui --test native_ui_smoke -- --ignored --test-threads=1
//! Optional IRONSETUP_UI_SMOKE_SCREENSHOTS records these test windows as BMPs.
#![cfg(windows)]

use anyhow::{anyhow, bail, Context, Result};
use ironsetup_core::*;
use ironsetup_ui::{compiler_studio, installer_wizard, WizardSelection};
use std::{
    collections::BTreeSet,
    fs,
    mem::size_of,
    path::{Path, PathBuf},
    ptr::null_mut,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::Threading::{GetCurrentProcessId, GetCurrentThreadId},
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
unsafe fn get_text(hwnd: HWND) -> String {
    let mut buffer = vec![0u16; GetWindowTextLengthW(hwnd).max(0) as usize + 1];
    let len = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32).max(0) as usize;
    String::from_utf16_lossy(&buffer[..len])
}
unsafe fn child(hwnd: HWND, id: i32) -> HWND {
    GetDlgItem(hwnd, id)
}
unsafe fn command(hwnd: HWND, id: usize, notify: usize) {
    PostMessageW(hwnd, WM_COMMAND, id | (notify << 16), 0);
}

unsafe extern "system" fn find_own_window(hwnd: HWND, output: LPARAM) -> BOOL {
    let mut process = 0;
    GetWindowThreadProcessId(hwnd, &mut process);
    let mut class = [0u16; 128];
    let len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
    if process == GetCurrentProcessId()
        && IsWindowVisible(hwnd) != 0
        && String::from_utf16_lossy(&class[..len.max(0) as usize]) == "IronSetup.Native.Window.v1"
    {
        *(output as *mut HWND) = hwnd;
        return 0;
    }
    1
}

fn wait_until<T>(description: &str, mut check: impl FnMut() -> Option<T>) -> Result<T> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(12) {
        if let Some(value) = check() {
            return Ok(value);
        }
        thread::sleep(Duration::from_millis(15));
    }
    bail!("Timed out waiting for {description}")
}
unsafe fn main_window() -> Result<HWND> {
    wait_until("the test window", || {
        let mut window: HWND = null_mut();
        EnumWindows(Some(find_own_window), (&mut window as *mut HWND) as isize);
        if window.is_null() {
            None
        } else {
            Some(window)
        }
    })
}
unsafe fn title_is(hwnd: HWND, value: &str) -> Result<()> {
    wait_until(value, || (get_text(child(hwnd, 11)) == value).then_some(()))
}
fn controller(
    ui_thread: u32,
    release: Arc<AtomicBool>,
    action: impl FnOnce() -> Result<()> + Send + 'static,
) -> thread::JoinHandle<Result<()>> {
    thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(action))
            .unwrap_or_else(|_| Err(anyhow!("UI controller panicked")));
        release.store(true, Ordering::Release);
        if result.is_err() {
            // Emergency cleanup is scoped to this test's UI thread, and is not
            // a production installer cancellation mechanism.
            unsafe {
                PostThreadMessageW(ui_thread, WM_QUIT, 1, 0);
            }
        }
        result
    })
}

fn fixture() -> Manifest {
    let choice = |name: &str, types: &[&str], flags: &[&str]| Choice {
        name: name.into(),
        description: name.into(),
        types: types.iter().map(|s| s.to_string()).collect(),
        flags: flags.iter().map(|s| s.to_string()).collect(),
        conditions: Conditions::default(),
    };
    let mut docs_task = choice("docs_shortcut", &[], &[]);
    docs_task.conditions.components = "docs".into();
    Manifest {
        format_version: FORMAT_VERSION,
        app: AppInfo { id: "ui-smoke".into(), name: "IronSetup UI Smoke".into(), version: "0.1".into(), publisher: "Local test fixture".into(), default_dir: "{localappdata}\\UiSmoke".into(), default_group: "UiSmoke".into(), privileges: "lowest".into(), uninstallable: true, compression: "zip".into(), architectures_allowed: String::new() },
        files: vec![ManifestFile { path: "sample.txt".into(), size: 9, sha256: sha256(b"test-only"), executable: false, flags: vec![], conditions: Conditions::default() }],
        dirs: vec![], components: vec![choice("main", &["full", "minimal"], &["fixed"]), choice("docs", &["full"], &[])],
        tasks: vec![choice("desktop", &[], &[]), docs_task, choice("optional", &[], &["unchecked"])],
        types: vec![SetupType { name: "full".into(), description: "Full".into(), custom: false }, SetupType { name: "minimal".into(), description: "Minimal".into(), custom: false }],
        icons: vec![], registry: vec![], run: vec![],
        license_text: Some("TEST LICENSE\nAccept this fixture license to test the next page.\n繁體中文 Unicode license text.".into()),
        info_before: Some("Local GUI smoke fixture. No real installation takes place.".into()),
        info_after: Some("AFTER-INSTALL INFORMATION retained.".into()),
    }
}

#[test]
#[ignore = "requires a Windows desktop or Wine with DISPLAY; explicitly run --ignored"]
fn native_wizard_and_studio_end_to_end() -> Result<()> {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let temporary =
        std::env::temp_dir().join(format!("ironsetup-ui-smoke-{}-{stamp}", std::process::id()));
    fs::create_dir(&temporary)?;
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(temporary.clone());
    let destination = temporary.join("Application with spaces");
    let ui_thread = unsafe { GetCurrentThreadId() };
    let received = Arc::new(Mutex::new(None::<WizardSelection>));
    let started = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let control_destination = destination.clone();
    let control_started = started.clone();
    let control_release = release.clone();
    let driver = controller(ui_thread, release.clone(), move || unsafe {
        let hwnd = main_window()?;
        title_is(hwnd, "Install IronSetup UI Smoke")?;
        command(hwnd, 27, 0);
        title_is(hwnd, "Before you install")?;
        command(hwnd, 27, 0);
        title_is(hwnd, "License agreement")?;
        assert_eq!(
            IsWindowEnabled(child(hwnd, 27)),
            0,
            "license must gate Next"
        );
        assert_ne!(
            IsWindowVisible(child(hwnd, 14)),
            0,
            "license checkbox must be visible"
        );
        assert!(!get_text(child(hwnd, 14)).is_empty());
        screenshot(hwnd, "wizard-license.bmp")?;
        // Exercise the independent command-path gate, not just disabled visuals.
        command(hwnd, 27, 0);
        let popup = wait_until("license-gate error dialog", || {
            let popup = FindWindowW(wide("#32770").as_ptr(), wide("IronSetup").as_ptr());
            if popup.is_null() {
                return None;
            }
            let mut process = 0;
            GetWindowThreadProcessId(popup, &mut process);
            (process == GetCurrentProcessId()).then_some(popup)
        })?;
        PostMessageW(popup, WM_COMMAND, IDOK as usize, 0);
        wait_until("license popup to close", || {
            (IsWindow(popup) == 0).then_some(())
        })?;
        title_is(hwnd, "License agreement")?;
        SendMessageW(child(hwnd, 14), BM_CLICK, 0, 0);
        assert_ne!(IsWindowEnabled(child(hwnd, 27)), 0);
        command(hwnd, 27, 0);
        title_is(hwnd, "Choose an installation folder")?;
        SetWindowTextW(
            child(hwnd, 16),
            wide(&control_destination.to_string_lossy()).as_ptr(),
        );
        command(hwnd, 27, 0);
        title_is(hwnd, "Choose components and tasks")?;
        assert_eq!(
            SendMessageW(child(hwnd, 21), LB_GETSEL, 1, 0),
            1,
            "Full initially selects docs"
        );
        SendMessageW(child(hwnd, 19), CB_SETCURSEL, 1, 0);
        command(hwnd, 19, CBN_SELCHANGE as usize);
        wait_until("minimal component selection", || {
            (SendMessageW(child(hwnd, 21), LB_GETSEL, 1, 0) == 0).then_some(())
        })?;
        assert_eq!(SendMessageW(child(hwnd, 21), LB_GETSEL, 0, 0), 1);
        assert_eq!(SendMessageW(child(hwnd, 23), LB_GETSEL, 0, 0), 1);
        assert_eq!(
            SendMessageW(child(hwnd, 23), LB_GETSEL, 1, 0),
            0,
            "conditional docs task deselected"
        );
        // A user cannot deselect the required component.
        SendMessageW(child(hwnd, 21), LB_SETSEL, 0, 0);
        command(hwnd, 21, LBN_SELCHANGE as usize);
        wait_until("fixed component restoration", || {
            (SendMessageW(child(hwnd, 21), LB_GETSEL, 0, 0) == 1).then_some(())
        })?;
        screenshot(hwnd, "wizard-features.bmp")?;
        command(hwnd, 27, 0);
        title_is(hwnd, "Ready to install")?;
        assert!(get_text(child(hwnd, 13)).contains("Selected files: 1"));
        command(hwnd, 27, 0);
        wait_until("installation callback", || {
            control_started.load(Ordering::Acquire).then_some(())
        })?;
        assert_eq!(IsWindowEnabled(child(hwnd, 28)), 0);
        PostMessageW(hwnd, WM_CLOSE, 0, 0);
        thread::sleep(Duration::from_millis(80));
        assert_ne!(IsWindow(hwnd), 0, "window must stay alive during commit");
        control_release.store(true, Ordering::Release);
        title_is(hwnd, "Installation complete")?;
        let body = get_text(child(hwnd, 13));
        assert!(body.contains("WARNING: smoke warning is preserved"));
        assert!(body.contains("AFTER-INSTALL INFORMATION"));
        screenshot(hwnd, "wizard-complete.bmp")?;
        command(hwnd, 27, 0);
        Ok(())
    });
    let callback_received = received.clone();
    let callback_release = release.clone();
    let wizard_result = installer_wizard(
        &fixture(),
        &destination,
        Box::new(move |selection, progress| {
            *callback_received
                .lock()
                .map_err(|_| anyhow!("poisoned selection mutex"))? = Some(selection);
            started.store(true, Ordering::Release);
            progress(Progress {
                stage: "Testing worker progress".into(),
                item: "test-only fixture".into(),
                completed: 1,
                total: 2,
            });
            wait_until("UI releasing the fixture callback", || {
                callback_release.load(Ordering::Acquire).then_some(())
            })?;
            progress(Progress {
                stage: "Fixture complete".into(),
                item: "test-only fixture".into(),
                completed: 2,
                total: 2,
            });
            Ok("TEST CALLBACK COMPLETE\nWARNING: smoke warning is preserved".into())
        }),
    );
    driver
        .join()
        .map_err(|_| anyhow!("wizard controller join failed"))??;
    wizard_result?;
    let selection = received
        .lock()
        .map_err(|_| anyhow!("poisoned selection mutex"))?
        .take()
        .context("callback did not receive selections")?;
    assert_eq!(selection.target_dir, destination);
    assert_eq!(selection.components, BTreeSet::from(["main".into()]));
    assert_eq!(selection.tasks, BTreeSet::from(["desktop".into()]));
    println!("PASS wizard license gate, destination, type, fixed components, conditional tasks, progress, busy close, result and warning text");

    let script = temporary.join("smoke.iss");
    fs::write(&script, "[Setup]\nAppName=Before\n")?;
    let changed_text = "[Setup]\r\nAppName=繁體中文 UI Smoke\r\nAppVersion=1.0\r\n";
    let build_started = Arc::new(AtomicBool::new(false));
    let build_release = Arc::new(AtomicBool::new(false));
    let control_started = build_started.clone();
    let control_release = build_release.clone();
    let controller_script = script.clone();
    let driver = controller(ui_thread, build_release.clone(), move || unsafe {
        let hwnd = main_window()?;
        wait_until("studio editor", || {
            (!child(hwnd, 108).is_null()).then_some(())
        })?;
        SetWindowTextW(child(hwnd, 108), wide(changed_text).as_ptr());
        command(hwnd, 102, 0);
        wait_until("UTF-8 script save", || {
            fs::read_to_string(&controller_script)
                .ok()
                .filter(|text| text == changed_text)
                .map(|_| ())
        })?;
        PostMessageW(hwnd, WM_KEYDOWN, VK_F9 as usize, 0);
        wait_until("compiler callback", || {
            control_started.load(Ordering::Acquire).then_some(())
        })?;
        assert_eq!(IsWindowEnabled(child(hwnd, 104)), 0);
        PostMessageW(hwnd, WM_CLOSE, 0, 0);
        thread::sleep(Duration::from_millis(80));
        assert_ne!(
            IsWindow(hwnd),
            0,
            "studio remains alive during the compiler callback"
        );
        control_release.store(true, Ordering::Release);
        wait_until("build completion log", || {
            get_text(child(hwnd, 110))
                .contains("SMOKE BUILD COMPLETE")
                .then_some(())
        })?;
        assert_ne!(IsWindowEnabled(child(hwnd, 104)), 0);
        assert!(get_text(child(hwnd, 110)).contains("SMOKE BUILD WARNING"));
        screenshot(hwnd, "studio-editor.bmp")?;
        PostMessageW(hwnd, WM_CLOSE, 0, 0);
        Ok(())
    });
    let callback_release = build_release.clone();
    let studio_result = compiler_studio(
        Some(script.clone()),
        Box::new(move |path| {
            assert!(fs::read_to_string(path)?.contains("繁體中文"));
            build_started.store(true, Ordering::Release);
            wait_until("UI releasing the compiler callback", || {
                callback_release.load(Ordering::Acquire).then_some(())
            })?;
            Ok("SMOKE BUILD COMPLETE\nSMOKE BUILD WARNING: this is a GUI fixture, not a real compilation".into())
        }),
    );
    driver
        .join()
        .map_err(|_| anyhow!("studio controller join failed"))??;
    studio_result?;
    assert_eq!(fs::read_to_string(script)?, changed_text);
    println!(
        "PASS studio UTF-8 edit/save, F9, threaded build, busy close, completion and warning log"
    );
    Ok(())
}

unsafe fn screenshot(hwnd: HWND, filename: &str) -> Result<()> {
    let Some(directory) = std::env::var_os("IRONSETUP_UI_SMOKE_SCREENSHOTS") else {
        return Ok(());
    };
    let directory = Path::new(&directory);
    fs::create_dir_all(directory)?;
    RedrawWindow(
        hwnd,
        std::ptr::null(),
        null_mut(),
        RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW,
    );
    let mut rect: RECT = std::mem::zeroed();
    if GetWindowRect(hwnd, &mut rect) == 0 {
        bail!("GetWindowRect failed");
    }
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 || width > 8192 || height > 8192 {
        bail!("Invalid screenshot dimensions");
    }
    let source = GetWindowDC(hwnd);
    if source.is_null() {
        bail!("Cannot capture test window DC");
    }
    let memory = CreateCompatibleDC(source);
    let bitmap = CreateCompatibleBitmap(source, width, height);
    if memory.is_null() || bitmap.is_null() {
        if !memory.is_null() {
            DeleteDC(memory);
        }
        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        ReleaseDC(hwnd, source);
        bail!("Cannot allocate test screenshot");
    }
    let previous = SelectObject(memory, bitmap);
    let copied = BitBlt(memory, 0, 0, width, height, source, 0, 0, SRCCOPY);
    SelectObject(memory, previous);
    let mut info: BITMAPINFO = std::mem::zeroed();
    info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = width;
    info.bmiHeader.biHeight = -height;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    let bytes = (width * height * 4) as usize;
    let mut pixels = vec![0u8; bytes];
    let lines = if copied != 0 {
        GetDIBits(
            source,
            bitmap,
            0,
            height as u32,
            pixels.as_mut_ptr().cast(),
            &mut info,
            DIB_RGB_COLORS,
        )
    } else {
        0
    };
    DeleteObject(bitmap);
    DeleteDC(memory);
    ReleaseDC(hwnd, source);
    if lines == 0 {
        bail!("Cannot read test screenshot pixels");
    }
    let mut result = Vec::with_capacity(54 + bytes);
    result.extend_from_slice(b"BM");
    result.extend_from_slice(&((54 + bytes) as u32).to_le_bytes());
    result.extend_from_slice(&[0u8; 4]);
    result.extend_from_slice(&54u32.to_le_bytes());
    result.extend_from_slice(&40u32.to_le_bytes());
    result.extend_from_slice(&width.to_le_bytes());
    result.extend_from_slice(&(-height).to_le_bytes());
    result.extend_from_slice(&1u16.to_le_bytes());
    result.extend_from_slice(&32u16.to_le_bytes());
    result.extend_from_slice(&[0u8; 4]);
    result.extend_from_slice(&(bytes as u32).to_le_bytes());
    result.extend_from_slice(&[0u8; 16]);
    result.extend_from_slice(&pixels);
    fs::write(directory.join(filename), result)?;
    Ok(())
}
