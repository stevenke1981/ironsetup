use crate::journal::{FileLocation, RegistryChange, RegistryValue};
use anyhow::{bail, Context, Result};
#[cfg(windows)]
use ironsetup_core::Package;
use ironsetup_core::{validate_relative_path, AppInfo, IconEntry, RegistryEntry, RunEntry};
use std::path::{Path, PathBuf};

pub(crate) struct ContextPaths {
    pub app: PathBuf,
    pub info: AppInfo,
    local: PathBuf,
    roaming: PathBuf,
    programs: PathBuf,
    desktop: PathBuf,
    userpf: PathBuf,
    pf: PathBuf,
}
impl ContextPaths {
    pub fn new(info: &AppInfo, app: Option<PathBuf>) -> Result<Self> {
        validate_relative_path(&info.default_group)
            .context("DefaultGroupName must be a safe relative folder name")?;
        let folders = known_folders()?;
        let userpf = folders.local.join("Programs");
        Ok(Self {
            app: app.unwrap_or_default(),
            info: info.clone(),
            local: folders.local,
            roaming: folders.roaming,
            programs: folders.programs,
            desktop: folders.desktop,
            userpf,
            pf: folders.pf,
        })
    }
    pub fn base(&self, root: &str) -> Result<&Path> {
        match root {
            "app" => Ok(&self.app),
            "desktop" => Ok(&self.desktop),
            "programs" => Ok(&self.programs),
            _ => bail!("Unknown journal path root: {root}"),
        }
    }
    pub fn expand(&self, input: &str) -> Result<String> {
        let mut output = String::new();
        let mut remaining = input;
        while let Some(index) = remaining.find('{') {
            output.push_str(&remaining[..index]);
            remaining = &remaining[index..];
            if remaining.starts_with("{{") {
                output.push('{');
                remaining = &remaining[2..];
                continue;
            }
            let end = remaining.find('}').context("Unclosed runtime constant")?;
            let constant = remaining[1..end].to_ascii_lowercase();
            let path = match constant.as_str() {
                "app" if !self.app.as_os_str().is_empty() => self.app.clone(),
                "uninstallexe" if !self.app.as_os_str().is_empty() => self.app.join("unins000.exe"),
                "group" => self
                    .programs
                    .join(self.info.default_group.replace('\\', "/")),
                "userdesktop" | "autodesktop" => self.desktop.clone(),
                "userprograms" => self.programs.clone(),
                "localappdata" => self.local.clone(),
                "userappdata" => self.roaming.clone(),
                "userpf" => self.userpf.clone(),
                "autopf" => {
                    if self.info.privileges == "admin" {
                        self.pf.clone()
                    } else {
                        self.userpf.clone()
                    }
                }
                "pf" | "pf64" => self.pf.clone(),
                _ => bail!("Unknown or unavailable runtime constant: {{{constant}}}"),
            };
            output.push_str(&shell_path(&path));
            remaining = &remaining[end + 1..];
        }
        output.push_str(remaining);
        if output.contains('\0') {
            bail!("NUL byte is not allowed in runtime strings");
        }
        Ok(output)
    }
    pub fn app_filename(&self, value: &str, allow_uninstaller: bool) -> Result<PathBuf> {
        if allow_uninstaller && value.eq_ignore_ascii_case("{uninstallexe}") {
            let result = self.app.join("unins000.exe");
            ironsetup_core::reject_symlink_ancestors(&result)?;
            return Ok(result);
        }
        match crate::app_relative(value)? {
            Some(relative) => ironsetup_core::checked_join(&self.app, &relative),
            None => Ok(self.app.clone()),
        }
    }
    pub fn shortcut_location(&self, name: &str) -> Result<FileLocation> {
        let end = name
            .find('}')
            .context("Shortcut name must start with a known folder constant")?;
        let root = name[..=end].to_ascii_lowercase();
        let remainder = name[end + 1..]
            .strip_prefix(['\\', '/'])
            .context("Shortcut name needs a relative filename")?;
        validate_relative_path(remainder)?;
        let (base, path) = match root.as_str() {
            "{group}" => (
                "programs",
                format!(
                    "{}/{}",
                    self.info.default_group.replace('\\', "/"),
                    remainder.replace('\\', "/")
                ),
            ),
            "{userprograms}" => ("programs", remainder.replace('\\', "/")),
            "{userdesktop}" | "{autodesktop}" => ("desktop", remainder.replace('\\', "/")),
            _ => bail!("Unsupported shortcut folder: {root}"),
        };
        let path = if path.to_ascii_lowercase().ends_with(".lnk") {
            path
        } else {
            format!("{path}.lnk")
        };
        let location = FileLocation {
            root: base.into(),
            path,
        };
        location.resolve_file(self)?;
        Ok(location)
    }
}
struct KnownFolders {
    local: PathBuf,
    roaming: PathBuf,
    programs: PathBuf,
    desktop: PathBuf,
    pf: PathBuf,
}

// Filesystem paths remain canonical internally. Explorer shortcuts and ARP
// command strings need conventional DOS/UNC spelling on Windows.
fn shell_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(unc) = value.strip_prefix("\\\\?\\UNC\\") {
            return format!("\\\\{unc}");
        }
        if let Some(local) = value.strip_prefix("\\\\?\\") {
            return local.to_string();
        }
    }
    value.into_owned()
}

#[cfg(not(windows))]
fn known_folders() -> Result<KnownFolders> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is unavailable")?;
    if !home.is_absolute() {
        bail!("HOME must be an absolute directory");
    }
    let local = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".local/share"));
    let roaming = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    Ok(KnownFolders {
        programs: local.join("applications"),
        desktop: home.join("Desktop"),
        pf: PathBuf::from("/opt"),
        local,
        roaming,
    })
}
#[cfg(windows)]
fn known_folders() -> Result<KnownFolders> {
    use windows::Win32::UI::Shell::*;
    fn get(id: &windows::core::GUID) -> Result<PathBuf> {
        unsafe {
            let value = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None)?;
            let string = value.to_string()?;
            windows::Win32::System::Com::CoTaskMemFree(Some(value.0.cast()));
            Ok(PathBuf::from(string))
        }
    }
    Ok(KnownFolders {
        local: get(&FOLDERID_LocalAppData)?,
        roaming: get(&FOLDERID_RoamingAppData)?,
        programs: get(&FOLDERID_Programs)?,
        desktop: get(&FOLDERID_Desktop)?,
        pf: get(&FOLDERID_ProgramFiles)?,
    })
}

pub(crate) fn check_privileges(app: &AppInfo) -> Result<()> {
    if app.privileges == "admin" {
        #[cfg(not(windows))]
        bail!("PrivilegesRequired=admin is supported only on Windows");
        #[cfg(windows)]
        if unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() } == 0 {
            bail!("This installer requires administrator privileges; restart with elevation");
        }
    }
    Ok(())
}

fn registry_root(root: &str, app: &AppInfo) -> Result<String> {
    match root.to_ascii_uppercase().as_str() {
        "HKCU" => Ok("HKCU".into()),
        "HKLM" if app.privileges == "admin" => Ok("HKLM".into()),
        "HKLM" => bail!("HKLM registry writes require PrivilegesRequired=admin"),
        "HKA" => Ok(if app.privileges == "admin" {
            "HKLM"
        } else {
            "HKCU"
        }
        .into()),
        _ => bail!("Unsupported registry root: {root}"),
    }
}
fn validate_subkey(subkey: &str) -> Result<()> {
    if subkey.is_empty()
        || subkey.len() > 16_384
        || subkey.contains(['\0', '/', '{', '}'])
        || subkey
            .split('\\')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        bail!("Invalid registry subkey: {subkey}");
    }
    Ok(())
}
pub(crate) fn validate_registry_entry(entry: &RegistryEntry, app: &AppInfo) -> Result<()> {
    registry_root(&entry.root, app)?;
    validate_subkey(&entry.subkey)?;
    if entry.value_name.contains('\0') || entry.value_name.len() > 16_384 {
        bail!("Invalid registry value name");
    }
    if entry.flags.iter().any(|f| f != "uninsdeletevalue") {
        bail!("Unsupported registry flags");
    }
    if !matches!(
        entry.value_type.as_str(),
        "string" | "expandsz" | "dword" | "qword" | "binary"
    ) {
        bail!("Unsupported registry value type: {}", entry.value_type);
    }
    if entry.value_data.len() > 1024 * 1024 {
        bail!("Registry value exceeds supported limit");
    }
    Ok(())
}
pub(crate) fn validate_registry_change(change: &RegistryChange, app: &AppInfo) -> Result<()> {
    let expected = registry_root(&change.root, app)?;
    if expected != change.root {
        bail!("Journal registry roots must be resolved");
    }
    validate_subkey(&change.subkey)?;
    if change.name.contains('\0')
        || change.name.len() > 16_384
        || change.installed.data.len() > 1024 * 1024
        || change
            .previous
            .as_ref()
            .is_some_and(|p| p.data.len() > 1024 * 1024)
    {
        bail!("Invalid registry journal data");
    }
    for key in &change.created_keys {
        validate_subkey(key)?;
        if key != &change.subkey
            && !change
                .subkey
                .to_ascii_lowercase()
                .starts_with(&(key.to_ascii_lowercase() + "\\"))
        {
            bail!("Registry cleanup key is outside the written key");
        }
    }
    Ok(())
}
pub(crate) fn prepare_registry_change(
    entry: &RegistryEntry,
    paths: &ContextPaths,
) -> Result<RegistryChange> {
    validate_registry_entry(entry, &paths.info)?;
    let root = registry_root(&entry.root, &paths.info)?;
    let data = paths.expand(&entry.value_data)?;
    let installed = encode_registry_value(&entry.value_type, &data)?;
    Ok(RegistryChange {
        previous: get_registry(&root, &entry.subkey, &entry.value_name)?,
        created_keys: missing_registry_keys(&root, &entry.subkey)?,
        root,
        subkey: entry.subkey.clone(),
        name: entry.value_name.clone(),
        installed,
        remove_on_uninstall: entry.flags.iter().any(|f| f == "uninsdeletevalue"),
        undo_done: false,
        retained_backup: false,
    })
}
fn encode_registry_value(kind: &str, value: &str) -> Result<RegistryValue> {
    let (kind, data) = match kind {
        "string" | "expandsz" => (
            if kind == "string" { 1 } else { 2 },
            value
                .encode_utf16()
                .chain(Some(0))
                .flat_map(u16::to_le_bytes)
                .collect(),
        ),
        "dword" => (
            4,
            value
                .parse::<u32>()
                .context("Invalid DWORD")?
                .to_le_bytes()
                .to_vec(),
        ),
        "qword" => (
            11,
            value
                .parse::<u64>()
                .context("Invalid QWORD")?
                .to_le_bytes()
                .to_vec(),
        ),
        "binary" => {
            let value = value
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>();
            if value.len() % 2 != 0 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                bail!("Invalid hexadecimal binary registry data");
            }
            let data = value
                .as_bytes()
                .chunks(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap_or(""), 16))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            (3, data)
        }
        _ => bail!("Unsupported registry value type"),
    };
    Ok(RegistryValue { kind, data })
}

#[cfg(not(windows))]
pub(crate) fn get_registry(_: &str, _: &str, _: &str) -> Result<Option<RegistryValue>> {
    bail!("Registry operations require Windows")
}
#[cfg(not(windows))]
fn missing_registry_keys(_: &str, _: &str) -> Result<Vec<String>> {
    bail!("Registry operations require Windows")
}
#[cfg(not(windows))]
pub(crate) fn set_registry(_: &str, _: &str, _: &str, _: &RegistryValue) -> Result<()> {
    bail!("Registry operations require Windows")
}
#[cfg(not(windows))]
pub(crate) fn delete_registry_value(_: &str, _: &str, _: &str) -> Result<()> {
    bail!("Registry operations require Windows")
}
#[cfg(not(windows))]
pub(crate) fn delete_empty_registry_key(_: &str, _: &str) -> Result<()> {
    bail!("Registry operations require Windows")
}

#[cfg(windows)]
mod registry_windows {
    use super::*;
    use windows_sys::Win32::Foundation::{
        ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS,
    };
    use windows_sys::Win32::System::Registry::*;
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    fn root(value: &str) -> Result<HKEY> {
        match value {
            "HKCU" => Ok(HKEY_CURRENT_USER),
            "HKLM" => Ok(HKEY_LOCAL_MACHINE),
            _ => bail!("Unknown resolved registry root"),
        }
    }
    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                RegCloseKey(self.0);
            }
        }
    }
    fn open(root_name: &str, subkey: &str, access: u32) -> Result<Option<Key>> {
        let mut key = std::ptr::null_mut();
        let status =
            unsafe { RegOpenKeyExW(root(root_name)?, wide(subkey).as_ptr(), 0, access, &mut key) };
        if matches!(status, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
            return Ok(None);
        }
        if status != ERROR_SUCCESS {
            bail!("Open registry key {root_name}\\{subkey}: Windows error {status}");
        }
        Ok(Some(Key(key)))
    }
    pub(crate) fn get(root_name: &str, subkey: &str, name: &str) -> Result<Option<RegistryValue>> {
        let Some(key) = open(root_name, subkey, KEY_QUERY_VALUE)? else {
            return Ok(None);
        };
        let name = wide(name);
        let mut kind = 0;
        let mut size = 0;
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if status != ERROR_SUCCESS {
            bail!("Read registry value length: Windows error {status}");
        }
        if size > 1024 * 1024 {
            bail!("Existing registry value exceeds supported limit");
        }
        let mut data = vec![0u8; size as usize];
        let status = unsafe {
            RegQueryValueExW(
                key.0,
                name.as_ptr(),
                std::ptr::null(),
                &mut kind,
                data.as_mut_ptr(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS {
            bail!("Read registry value: Windows error {status}");
        }
        data.truncate(size as usize);
        Ok(Some(RegistryValue { kind, data }))
    }
    pub(crate) fn missing(root_name: &str, subkey: &str) -> Result<Vec<String>> {
        let mut prefix = String::new();
        let mut result = Vec::new();
        for part in subkey.split('\\') {
            if !prefix.is_empty() {
                prefix.push('\\');
            }
            prefix.push_str(part);
            if open(root_name, &prefix, KEY_QUERY_VALUE)?.is_none() {
                result.push(prefix.clone());
            }
        }
        Ok(result)
    }
    pub(crate) fn set(
        root_name: &str,
        subkey: &str,
        name: &str,
        value: &RegistryValue,
    ) -> Result<()> {
        let mut key = std::ptr::null_mut();
        let mut disposition = 0;
        let status = unsafe {
            RegCreateKeyExW(
                root(root_name)?,
                wide(subkey).as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut key,
                &mut disposition,
            )
        };
        if status != ERROR_SUCCESS {
            bail!("Create registry key: Windows error {status}");
        }
        let key = Key(key);
        let status = unsafe {
            RegSetValueExW(
                key.0,
                wide(name).as_ptr(),
                0,
                value.kind,
                value.data.as_ptr(),
                value.data.len().try_into()?,
            )
        };
        if status != ERROR_SUCCESS {
            bail!("Set registry value: Windows error {status}");
        }
        Ok(())
    }
    pub(crate) fn delete_value(root_name: &str, subkey: &str, name: &str) -> Result<()> {
        let Some(key) = open(root_name, subkey, KEY_SET_VALUE)? else {
            return Ok(());
        };
        let status = unsafe { RegDeleteValueW(key.0, wide(name).as_ptr()) };
        if !matches!(status, ERROR_SUCCESS | ERROR_FILE_NOT_FOUND) {
            bail!("Delete registry value: Windows error {status}");
        }
        Ok(())
    }
    pub(crate) fn delete_empty(root_name: &str, subkey: &str) -> Result<()> {
        let Some(key) = open(root_name, subkey, KEY_QUERY_VALUE | KEY_ENUMERATE_SUB_KEYS)? else {
            return Ok(());
        };
        let mut subkeys = 0;
        let mut values = 0;
        let status = unsafe {
            RegQueryInfoKeyW(
                key.0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
                &mut subkeys,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut values,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if status != ERROR_SUCCESS {
            bail!("Inspect registry key: Windows error {status}");
        }
        drop(key);
        if subkeys != 0 || values != 0 {
            return Ok(());
        }
        let status = unsafe { RegDeleteKeyExW(root(root_name)?, wide(subkey).as_ptr(), 0, 0) };
        if !matches!(
            status,
            ERROR_SUCCESS | ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND
        ) {
            bail!("Delete empty registry key: Windows error {status}");
        }
        Ok(())
    }
}
#[cfg(windows)]
use registry_windows::missing as missing_registry_keys;
#[cfg(windows)]
pub(crate) use registry_windows::{
    delete_empty as delete_empty_registry_key, delete_value as delete_registry_value,
    get as get_registry, set as set_registry,
};

#[cfg(not(windows))]
pub(crate) fn shortcut_bytes(_: &IconEntry, _: &ContextPaths, _: &Path) -> Result<Vec<u8>> {
    bail!("Native shortcut creation requires Windows")
}
#[cfg(windows)]
pub(crate) fn shortcut_bytes(
    icon: &IconEntry,
    paths: &ContextPaths,
    state: &Path,
) -> Result<Vec<u8>> {
    use windows::{
        core::{Interface, PCWSTR},
        Win32::{System::Com::*, UI::Shell::*},
    };
    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }
    struct ComGuard(bool);
    impl Drop for ComGuard {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() }
            }
        }
    }
    unsafe {
        let initialization = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let _guard = ComGuard(initialization.is_ok());
        if initialization.is_err() && initialization.0 != 0x80010106u32 as i32 {
            initialization.ok()?;
        }
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let target = wide(&shell_path(&paths.app_filename(&icon.filename, true)?));
        link.SetPath(PCWSTR(target.as_ptr()))?;
        let arguments = wide(&paths.expand(&icon.parameters)?);
        link.SetArguments(PCWSTR(arguments.as_ptr()))?;
        let working = if icon.working_dir.is_empty() {
            paths.app.clone()
        } else {
            paths.app_filename(&icon.working_dir, false)?
        };
        let working = wide(&shell_path(&working));
        link.SetWorkingDirectory(PCWSTR(working.as_ptr()))?;
        if !icon.comment.is_empty() {
            let comment = wide(&icon.comment);
            link.SetDescription(PCWSTR(comment.as_ptr()))?;
        }
        if !icon.icon_filename.is_empty() {
            let filename = wide(&shell_path(
                &paths.app_filename(&icon.icon_filename, false)?,
            ));
            link.SetIconLocation(PCWSTR(filename.as_ptr()), 0)?;
        }
        let temporary = tempfile::Builder::new()
            .prefix("shortcut-")
            .suffix(".lnk")
            .tempfile_in(state)?
            .into_temp_path();
        let filename = wide(&temporary.to_string_lossy());
        let persistent: IPersistFile = link.cast()?;
        persistent.Save(PCWSTR(filename.as_ptr()), true)?;
        Ok(std::fs::read(&temporary)?)
    }
}

#[cfg(windows)]
pub(crate) fn uninstall_registration(
    app: &AppInfo,
    target: &Path,
    package: &Package,
) -> Result<Vec<RegistryEntry>> {
    use ironsetup_core::{sha256, Conditions};
    let root = if app.privileges == "admin" {
        "HKLM"
    } else {
        "HKCU"
    };
    let subkey = format!(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\IronSetup_{}",
        &sha256(app.id.as_bytes())[..32]
    );
    let executable = target.join("unins000.exe");
    let uninstall = format!(
        "\"{}\" /UNINSTALL /DIR=\"{}\"",
        shell_path(&executable),
        shell_path(target)
    );
    let size_kib = package
        .manifest
        .files
        .iter()
        .map(|f| f.size)
        .sum::<u64>()
        .div_ceil(1024)
        .min(u32::MAX as u64);
    let items = [
        ("DisplayName", "string", app.name.clone()),
        ("DisplayVersion", "string", app.version.clone()),
        ("Publisher", "string", app.publisher.clone()),
        ("InstallLocation", "string", shell_path(target)),
        ("UninstallString", "string", uninstall.clone()),
        (
            "QuietUninstallString",
            "string",
            format!("{uninstall} /VERYSILENT"),
        ),
        ("NoModify", "dword", "1".into()),
        ("NoRepair", "dword", "1".into()),
        ("EstimatedSize", "dword", size_kib.to_string()),
    ];
    Ok(items
        .into_iter()
        .map(|(name, kind, value)| RegistryEntry {
            root: root.into(),
            subkey: subkey.clone(),
            value_type: kind.into(),
            value_name: name.into(),
            value_data: value.replace('{', "{{"),
            flags: vec!["uninsdeletevalue".into()],
            conditions: Conditions::default(),
        })
        .collect())
}

pub(crate) fn run_program(entry: &RunEntry, paths: &ContextPaths) -> Result<Option<i32>> {
    let executable = paths.app_filename(&entry.filename, false)?;
    if !executable.is_file() {
        bail!(
            "Program is missing or not a regular file: {}",
            executable.display()
        );
    }
    let working = if entry.working_dir.is_empty() {
        paths.app.clone()
    } else {
        paths.app_filename(&entry.working_dir, false)?
    };
    let parameters = paths.expand(&entry.parameters)?;
    let mut command = std::process::Command::new(&executable);
    command.current_dir(working);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.raw_arg(parameters);
        if entry.flags.iter().any(|f| f == "runhidden") {
            command.creation_flags(0x08000000);
        }
    }
    #[cfg(not(windows))]
    {
        if entry.flags.iter().any(|f| f == "runhidden") {
            bail!("Run flag runhidden requires Windows");
        }
        command.args(split_arguments(&parameters)?);
    }
    if entry.flags.iter().any(|f| f == "nowait") {
        command.spawn()?;
        return Ok(None);
    }
    let status = command.status()?;
    Ok(Some(status.code().unwrap_or(-1)))
}

#[cfg(not(windows))]
fn split_arguments(value: &str) -> Result<Vec<String>> {
    // Windows-style double-quote/backslash argument splitting for portable
    // filesystem tests. No shell evaluation, interpolation, or glob expansion.
    let mut result = Vec::new();
    let mut chars = value.chars().peekable();
    while chars.peek().is_some() {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let mut argument = String::new();
        let mut quoted = false;
        loop {
            match chars.peek().copied() {
                None => break,
                Some(c) if c.is_whitespace() && !quoted => break,
                Some('\\') => {
                    let mut count = 0;
                    while chars.peek() == Some(&'\\') {
                        chars.next();
                        count += 1;
                    }
                    if chars.peek() == Some(&'"') {
                        argument.extend(std::iter::repeat_n('\\', count / 2));
                        chars.next();
                        if count % 2 == 1 {
                            argument.push('"');
                        } else {
                            quoted = !quoted;
                        }
                    } else {
                        argument.extend(std::iter::repeat_n('\\', count));
                    }
                }
                Some('"') => {
                    chars.next();
                    quoted = !quoted;
                }
                Some(c) => {
                    chars.next();
                    argument.push(c);
                }
            }
        }
        if quoted {
            bail!("Unclosed quote in Run.Parameters");
        }
        result.push(argument);
    }
    Ok(result)
}
