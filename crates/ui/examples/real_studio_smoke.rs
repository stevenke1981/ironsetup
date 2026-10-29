//! Opt-in integration driver for actual release binaries, not injected callbacks.
//! Arguments: studio.exe ironsetup.exe script.iss output.exe temporary-install-dir
#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(not(windows))]
fn main() {
    eprintln!("This opt-in integration driver requires Windows or Wine.");
}

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    native::run()
}

#[cfg(windows)]
mod native {
    use anyhow::{anyhow, bail, ensure, Context, Result};
    use std::{
        fs,
        path::PathBuf,
        process::{Child, Command},
        ptr::null_mut,
        thread,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::*,
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    };

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    unsafe fn text(hwnd: HWND) -> String {
        // GetWindowTextW cannot read another process's edit control. These are
        // system messages and Win32 marshals the UTF-16 buffer across processes.
        let length = SendMessageW(hwnd, WM_GETTEXTLENGTH, 0, 0).clamp(0, 2_000_000) as usize;
        let mut buffer = vec![0u16; length + 1];
        let actual = SendMessageW(hwnd, WM_GETTEXT, buffer.len(), buffer.as_mut_ptr() as isize)
            .clamp(0, length as isize) as usize;
        String::from_utf16_lossy(&buffer[..actual])
    }
    unsafe extern "system" fn find(hwnd: HWND, data: LPARAM) -> BOOL {
        let data = &mut *(data as *mut (u32, HWND));
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let mut class = [0u16; 128];
        let length = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
        if pid == data.0
            && IsWindowVisible(hwnd) != 0
            && String::from_utf16_lossy(&class[..length]) == "IronSetup.Native.Window.v1"
        {
            data.1 = hwnd;
            return 0;
        }
        1
    }
    fn wait<T>(label: &str, mut condition: impl FnMut() -> Result<Option<T>>) -> Result<T> {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(45) {
            if let Some(value) = condition()? {
                return Ok(value);
            }
            thread::sleep(Duration::from_millis(25));
        }
        bail!("Timeout: {label}")
    }
    unsafe fn window(child: &mut Child) -> Result<HWND> {
        wait("spawned process window", || {
            if let Some(code) = child.try_wait()? {
                bail!("GUI exited before opening: {code}");
            }
            let mut data: (u32, HWND) = (child.id(), null_mut());
            EnumWindows(Some(find), (&mut data as *mut (u32, HWND)) as isize);
            Ok((!data.1.is_null()).then_some(data.1))
        })
    }
    unsafe fn page(hwnd: HWND, child: &mut Child, expected: &str) -> Result<()> {
        wait(expected, || {
            if let Some(code) = child.try_wait()? {
                bail!("GUI exited while waiting for {expected}: {code}");
            }
            let title = text(GetDlgItem(hwnd, 11));
            if title == "Installation did not complete" {
                bail!("Installer reported failure: {}", text(GetDlgItem(hwnd, 13)));
            }
            Ok((title == expected).then_some(()))
        })
    }
    unsafe fn next(hwnd: HWND) {
        // Exercise the real button path after the page's native paint/focus
        // messages settle, as a person clicking Next would do.
        thread::sleep(Duration::from_millis(150));
        SendMessageW(GetDlgItem(hwnd, 27), BM_CLICK, 0, 0);
    }
    fn exited(child: &mut Child) -> Result<()> {
        let status = wait("process exit", || Ok(child.try_wait()?))?;
        ensure!(status.success(), "Process exited with {status}");
        Ok(())
    }

    pub fn run() -> Result<()> {
        let args = std::env::args_os()
            .skip(1)
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        ensure!(
            args.len() == 5,
            "Expected studio.exe ironsetup.exe script.iss output.exe temporary-install-dir"
        );
        let (studio, cli, script, output, install) =
            (&args[0], &args[1], &args[2], &args[3], &args[4]);
        ensure!(
            !output.exists(),
            "Output already exists; test must prove a new build"
        );
        ensure!(
            !install.exists(),
            "Install directory already exists; test requires an isolated target"
        );
        let mut process = Command::new(studio)
            .arg(script)
            .spawn()
            .context("Start actual release Studio")?;
        let build_result = (|| unsafe {
            let hwnd = window(&mut process)?;
            PostMessageW(hwnd, WM_KEYDOWN, VK_F9 as usize, 0);
            let log = wait("actual Studio build result", || {
                if let Some(code) = process.try_wait()? {
                    bail!("Studio exited before build result: {code}");
                }
                let log = text(GetDlgItem(hwnd, 110));
                if log.contains("BUILD FAILED") {
                    bail!("{log}");
                }
                Ok(log.contains("Build succeeded").then_some(log))
            })?;
            println!("REAL STUDIO BUILD LOG:\n{log}");
            ensure!(
                output.is_file(),
                "Studio reported success without the expected installer"
            );
            ensure!(
                fs::metadata(output)?.len() > 100_000,
                "Installer is unexpectedly small"
            );
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
            exited(&mut process)
        })();
        if build_result.is_err() {
            let _ = process.kill();
            let _ = process.wait();
        }
        build_result?;
        let inspected = Command::new(cli).arg("inspect").arg(output).output()?;
        ensure!(
            inspected.status.success(),
            "Actual CLI inspect failed: {}",
            String::from_utf8_lossy(&inspected.stderr)
        );
        let manifest = String::from_utf8(inspected.stdout)?;
        ensure!(
            manifest.contains("Studio Integration Smoke"),
            "Wrong manifest in generated installer"
        );
        println!(
            "PASS real Studio F9 produced a verified single-EXE installer ({} bytes)",
            fs::metadata(output)?.len()
        );
        let inspection_path = script
            .parent()
            .ok_or_else(|| anyhow!("Missing script directory"))?
            .join("inspected-manifest.json");
        fs::write(inspection_path, manifest)?;

        let mut installer = Command::new(output)
            .arg(format!("/DIR={}", install.display()))
            .spawn()
            .context("Start generated real installer")?;
        let install_result = (|| unsafe {
            let hwnd = window(&mut installer)?;
            page(hwnd, &mut installer, "Install Studio Integration Smoke")?;
            next(hwnd);
            page(hwnd, &mut installer, "License agreement")?;
            ensure!(
                IsWindowEnabled(GetDlgItem(hwnd, 27)) == 0,
                "Real installer skipped license gate"
            );
            SendMessageW(GetDlgItem(hwnd, 14), BM_CLICK, 0, 0);
            next(hwnd);
            page(hwnd, &mut installer, "Choose an installation folder")?;
            SetWindowTextW(
                GetDlgItem(hwnd, 16),
                wide(&install.to_string_lossy()).as_ptr(),
            );
            next(hwnd);
            page(hwnd, &mut installer, "Ready to install")?;
            next(hwnd);
            page(hwnd, &mut installer, "Installation complete")?;
            println!("REAL INSTALLER RESULT:\n{}", text(GetDlgItem(hwnd, 13)));
            next(hwnd);
            exited(&mut installer)
        })();
        if install_result.is_err() {
            let _ = installer.kill();
            let _ = installer.wait();
        }
        install_result?;
        ensure!(
            fs::read_to_string(install.join("hello.txt"))?
                == "Real Studio / real SFX / real install.\n",
            "Installed payload differs from source"
        );
        ensure!(
            install.join("unins000.exe").is_file(),
            "Real installer did not write the uninstaller"
        );
        let verified = Command::new(cli)
            .arg("verify")
            .arg("--dir")
            .arg(install)
            .output()?;
        ensure!(
            verified.status.success(),
            "Installed-state verification failed: {}",
            String::from_utf8_lossy(&verified.stderr)
        );
        println!(
            "REAL VERIFY RESULT:\n{}",
            String::from_utf8_lossy(&verified.stdout)
        );
        let removed = Command::new(cli)
            .arg("uninstall")
            .arg("--dir")
            .arg(install)
            .output()?;
        ensure!(
            removed.status.success(),
            "Actual uninstall failed: {}",
            String::from_utf8_lossy(&removed.stderr)
        );
        ensure!(
            !install.exists(),
            "Real uninstall left the isolated target behind"
        );
        println!("PASS generated real SFX wizard installed exact payload and real CLI verify/uninstall removed its isolated target");
        Ok(())
    }
}
