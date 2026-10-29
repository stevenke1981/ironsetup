use crate::SetupArgs;
use anyhow::Result;
use std::path::Path;

#[cfg(not(windows))]
pub fn elevate_if_needed(_exe: &Path, _args: &[String]) -> Result<Option<i32>> {
    anyhow::bail!("Administrative Windows installation is unavailable on this platform")
}
#[cfg(not(windows))]
pub fn wait_for_parent(_pid: u32) -> Result<()> {
    Ok(())
}
#[cfg(not(windows))]
pub fn relaunch_uninstaller_from_temp(
    _exe: &Path,
    _target: &Path,
    _args: &SetupArgs,
) -> Result<bool> {
    Ok(false)
}

#[cfg(windows)]
mod native {
    use super::*;
    use anyhow::{ensure, Context};
    use std::{
        os::windows::{ffi::OsStrExt, process::CommandExt},
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY},
        System::Threading::{
            GetCurrentProcess, GetExitCodeProcess, OpenProcess, OpenProcessToken,
            WaitForSingleObject, CREATE_NO_WINDOW, INFINITE,
        },
        UI::{
            Shell::{
                ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
            },
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    };
    fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
        s.encode_wide().chain(Some(0)).collect()
    }
    fn elevated() -> Result<bool> {
        unsafe {
            let mut token = ptr::null_mut();
            ensure!(
                OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) != 0,
                "Cannot inspect process token: {}",
                std::io::Error::last_os_error()
            );
            let mut elevation: TOKEN_ELEVATION = std::mem::zeroed();
            let mut needed = 0;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                &mut elevation as *mut _ as *mut _,
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut needed,
            );
            CloseHandle(token);
            ensure!(
                ok != 0,
                "Cannot read process elevation: {}",
                std::io::Error::last_os_error()
            );
            Ok(elevation.TokenIsElevated != 0)
        }
    }
    pub fn elevate_if_needed(exe: &Path, args: &[String]) -> Result<Option<i32>> {
        if elevated()? {
            return Ok(None);
        }
        let file = wide(exe.as_os_str());
        let verb = wide(std::ffi::OsStr::new("runas"));
        let params = wide(std::ffi::OsStr::new(
            &args
                .iter()
                .map(|s| super::quote_arg(s))
                .collect::<Vec<_>>()
                .join(" "),
        ));
        unsafe {
            let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
            info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
            info.lpFile = file.as_ptr();
            info.lpVerb = verb.as_ptr();
            info.lpParameters = params.as_ptr();
            info.nShow = SW_SHOWNORMAL;
            ensure!(
                ShellExecuteExW(&mut info) != 0,
                "Administrator elevation was cancelled or failed: {}",
                std::io::Error::last_os_error()
            );
            ensure!(
                !info.hProcess.is_null(),
                "Elevated process handle unavailable"
            );
            WaitForSingleObject(info.hProcess, INFINITE);
            let mut code = 1;
            let ok = GetExitCodeProcess(info.hProcess, &mut code);
            CloseHandle(info.hProcess);
            ensure!(ok != 0, "Cannot read elevated installer result");
            Ok(Some(code as i32))
        }
    }
    pub fn wait_for_parent(pid: u32) -> Result<()> {
        unsafe {
            ensure!(
                pid != 0 && pid != std::process::id(),
                "Invalid parent process"
            );
            let process = OpenProcess(0x00100000, 0, pid);
            if process.is_null() {
                return Ok(());
            }
            let result = WaitForSingleObject(process, 30_000);
            CloseHandle(process);
            ensure!(
                result == 0,
                "Original uninstaller did not exit in 30 seconds"
            );
            Ok(())
        }
    }
    pub fn relaunch_uninstaller_from_temp(
        exe: &Path,
        target: &Path,
        args: &SetupArgs,
    ) -> Result<bool> {
        if args.parent_pid.is_some() || exe.parent() != Some(target) {
            return Ok(false);
        }
        use std::io::Write;
        let mut temp = tempfile::Builder::new()
            .prefix("ironsetup-uninstall-")
            .suffix(".exe")
            .tempfile()?;
        temp.write_all(&ironsetup_core::stub_bytes(exe)?)?;
        temp.as_file().sync_all()?;
        let (file, path) = temp.keep().map_err(|e| e.error)?;
        drop(file);
        let mut cmd = std::process::Command::new(&path);
        cmd.args(["--uninstall", "--confirmed", "--dir"])
            .arg(target)
            .arg("--parent-pid")
            .arg(std::process::id().to_string())
            .creation_flags(CREATE_NO_WINDOW);
        if args.silent {
            cmd.arg("--silent");
        }
        if let Some(log) = &args.log {
            match log {
                Some(p) => {
                    cmd.arg("--log").arg(p);
                }
                None => {
                    cmd.arg("/LOG");
                }
            }
        }
        if let Err(error) = cmd.spawn() {
            let _ = std::fs::remove_file(&path);
            return Err(error).context("Cannot start temporary uninstaller");
        }
        Ok(true)
    }
}
#[cfg(windows)]
pub use native::*;

#[cfg(any(windows, test))]
fn quote_arg(arg: &str) -> String {
    let mut out = String::from("\"");
    let mut slashes = 0;
    for c in arg.chars() {
        if c == '\\' {
            slashes += 1;
        } else {
            if c == '"' {
                out.extend(std::iter::repeat_n('\\', slashes * 2 + 1));
            } else {
                out.extend(std::iter::repeat_n('\\', slashes));
            }
            out.push(c);
            slashes = 0;
        }
    }
    out.extend(std::iter::repeat_n('\\', slashes * 2));
    out.push('"');
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quotes_windows_arguments() {
        assert_eq!(quote_arg("hello world"), "\"hello world\"");
        assert_eq!(quote_arg("C:\\tail\\"), "\"C:\\tail\\\\\"");
        assert_eq!(quote_arg("a\"b"), "\"a\\\"b\"");
    }
}
