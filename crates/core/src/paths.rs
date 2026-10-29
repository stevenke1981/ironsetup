use anyhow::{bail, Result};
use std::path::{Path, PathBuf};
pub fn validate_relative_path(path: &str) -> Result<()> {
    if path.encode_utf16().count() > 32_000 {
        bail!("Path is too long");
    }
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') {
        bail!("Invalid relative path: {path}");
    }
    for part in path.split(['/', '\\']) {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.encode_utf16().count() > 255
            || part.ends_with(['.', ' '])
            || part.chars().any(|c| c < ' ' || "<>:\"|?*".contains(c))
        {
            bail!("Unsafe path: {path}");
        }
        let upper = part.split('.').next().unwrap_or("").to_uppercase();
        if matches!(
            upper.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || ((upper.starts_with("COM") || upper.starts_with("LPT"))
            && matches!(
                upper.get(3..),
                Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")
            ))
        {
            bail!("Windows device name is not allowed: {path}");
        }
        if part.eq_ignore_ascii_case(crate::STATE_DIR) || part.eq_ignore_ascii_case("unins000.exe")
        {
            bail!("Installer metadata path is reserved: {path}");
        }
    }
    Ok(())
}
pub fn reject_symlink_ancestors(path: &Path) -> Result<()> {
    for p in path.ancestors() {
        match std::fs::symlink_metadata(p) {
            Ok(m) => {
                if m.file_type().is_symlink() {
                    bail!("Symbolic link is not allowed: {}", p.display());
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if m.file_attributes() & 0x400 != 0 {
                        bail!("Reparse point is not allowed: {}", p.display());
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
pub fn checked_join(root: &Path, relative: &str) -> Result<PathBuf> {
    validate_relative_path(relative)?;
    let target = root.join(relative.replace('\\', "/"));
    reject_symlink_ancestors(&target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guards_windows_path_forms() {
        for p in [
            "../x",
            "C:\\x",
            "/root",
            "a/../x",
            "a//b",
            "NUL.txt",
            "con",
            "a:ads",
            "a. ",
            "a.",
            "a/COM1.dat",
            "COM¹.txt",
            "a\\..\\b",
            "unins000.exe",
            ".ironsetup/state.json",
        ] {
            assert!(validate_relative_path(p).is_err(), "{p}");
        }
        for p in ["bin/app.exe", "資料/說明.txt", "LPT10.txt"] {
            validate_relative_path(p).unwrap();
        }
    }
    #[cfg(unix)]
    #[test]
    fn refuses_symlink_destination() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
        assert!(checked_join(dir.path(), "link/file").is_err());
    }
}
