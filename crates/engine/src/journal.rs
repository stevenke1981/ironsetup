use crate::{platform, UninstallReport};
use anyhow::{bail, Context, Result};
use ironsetup_core::{
    reject_symlink_ancestors, sha256, validate_relative_path, AppInfo, Progress, RegistryEntry,
    MAX_FILES, MAX_PACKAGE_BYTES,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_JOURNAL_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileLocation {
    pub root: String,
    pub path: String,
}
impl FileLocation {
    pub fn app(path: String) -> Self {
        Self {
            root: "app".into(),
            path,
        }
    }
    pub fn uninstaller() -> Self {
        Self {
            root: "uninstaller".into(),
            path: "unins000.exe".into(),
        }
    }
    pub fn resolve(&self, paths: &platform::ContextPaths) -> Result<PathBuf> {
        #[cfg(not(windows))]
        if matches!(self.root.as_str(), "desktop" | "programs") {
            bail!("External shortcut journal locations require Windows");
        }
        let value = if self.root == "uninstaller" {
            if self.path != "unins000.exe" {
                bail!("Invalid uninstaller location");
            }
            paths.app.join("unins000.exe")
        } else {
            validate_relative_path(&self.path)?;
            paths.base(&self.root)?.join(self.path.replace('\\', "/"))
        };
        reject_symlink_ancestors(&value)?;
        Ok(value)
    }
    pub fn resolve_file(&self, paths: &platform::ContextPaths) -> Result<PathBuf> {
        if matches!(self.root.as_str(), "desktop" | "programs")
            && !self.path.to_ascii_lowercase().ends_with(".lnk")
        {
            bail!("External journal files must be Windows .lnk shortcuts");
        }
        self.resolve(paths)
    }
    fn parent(&self) -> Option<Self> {
        if self.root == "uninstaller" {
            return None;
        }
        self.path
            .replace('\\', "/")
            .rsplit_once('/')
            .map(|(p, _)| Self {
                root: self.root.clone(),
                path: p.into(),
            })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OriginalFile {
    pub backup: String,
    pub sha256: String,
    pub permissions: FilePermissions,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FilePermissions {
    pub readonly: bool,
    pub unix_mode: Option<u32>,
}
impl FilePermissions {
    fn capture(path: &Path) -> Result<Self> {
        let permissions = fs::metadata(path)?.permissions();
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            Some(permissions.mode())
        };
        #[cfg(not(unix))]
        let mode = None;
        Ok(Self {
            readonly: permissions.readonly(),
            unix_mode: mode,
        })
    }
    fn payload(executable: bool) -> Self {
        Self {
            readonly: false,
            unix_mode: Some(if executable { 0o755 } else { 0o644 }),
        }
    }
    fn apply(&self, path: &Path) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = self.unix_mode {
                fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777))?;
            }
        }
        #[cfg(not(unix))]
        {
            let mut permissions = fs::metadata(path)?.permissions();
            permissions.set_readonly(self.readonly);
            fs::set_permissions(path, permissions)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileChange {
    pub location: FileLocation,
    pub installed_sha256: String,
    pub original: Option<OriginalFile>,
    pub retain_on_uninstall: bool,
    #[serde(default)]
    pub undo_done: bool,
    #[serde(default)]
    pub retained_backup: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegistryValue {
    pub kind: u32,
    pub data: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegistryChange {
    pub root: String,
    pub subkey: String,
    pub name: String,
    pub previous: Option<RegistryValue>,
    pub installed: RegistryValue,
    pub created_keys: Vec<String>,
    pub remove_on_uninstall: bool,
    #[serde(default)]
    pub undo_done: bool,
    #[serde(default)]
    pub retained_backup: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum JournalStatus {
    Installing,
    Installed,
    RollingBack,
    Uninstalling,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Journal {
    pub version: u32,
    pub app: AppInfo,
    pub target: PathBuf,
    pub target_created: bool,
    pub status: JournalStatus,
    pub files: Vec<FileChange>,
    pub directories: Vec<FileLocation>,
    pub registry: Vec<RegistryChange>,
}
impl Journal {
    pub fn new(app: AppInfo, target: PathBuf, target_created: bool) -> Self {
        Self {
            version: 1,
            app,
            target,
            target_created,
            status: JournalStatus::Installing,
            files: Vec::new(),
            directories: Vec::new(),
            registry: Vec::new(),
        }
    }
    pub fn save(&self, state: &Path) -> Result<()> {
        reject_symlink_ancestors(state)?;
        let bytes = serde_json::to_vec_pretty(self)?;
        if bytes.len() as u64 > MAX_JOURNAL_BYTES {
            bail!("Installation journal exceeds size limit");
        }
        let destination = state.join("journal.json");
        reject_symlink_ancestors(&destination)?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".journal-")
            .tempfile_in(state)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist(&destination).map_err(|e| e.error)?;
        sync_directory(state)?;
        Ok(())
    }
    pub fn load(state: &Path, target: &Path) -> Result<Self> {
        let file = state.join("journal.json");
        reject_symlink_ancestors(&file)?;
        let metadata =
            fs::metadata(&file).context("No valid IronSetup installation journal was found")?;
        if !metadata.is_file() || metadata.len() > MAX_JOURNAL_BYTES {
            bail!("Invalid or oversized installation journal");
        }
        let journal: Self = serde_json::from_slice(&fs::read(file)?)
            .context("Invalid installation journal; no changes made")?;
        if journal.version != 1 || journal.target != target {
            bail!("Installation journal does not match this directory");
        }
        Ok(journal)
    }
    pub fn validate(&self, state: &Path, paths: &platform::ContextPaths) -> Result<()> {
        if self.files.len() > MAX_FILES + 1000 || self.registry.len() > MAX_FILES {
            bail!("Unreasonable journal record count");
        }
        let mut seen = HashSet::new();
        for file in &self.files {
            let path = file.location.resolve_file(paths)?;
            if !seen.insert(path.to_string_lossy().to_lowercase()) {
                bail!("Duplicate journal file destination");
            }
            if !valid_digest(&file.installed_sha256) {
                bail!("Invalid file digest in journal");
            }
            if let Some(original) = &file.original {
                let backup = backup_path(state, &original.backup)?;
                if !valid_digest(&original.sha256) {
                    bail!("Invalid original digest");
                }
                if !file.undo_done {
                    let data = bounded_read(&backup)?;
                    if sha256(&data) != original.sha256 {
                        bail!("Original backup is corrupt: {}", backup.display());
                    }
                }
            }
        }
        for directory in &self.directories {
            if directory.root == "uninstaller" {
                bail!("Uninstaller is not a directory journal location");
            }
            directory.resolve(paths)?;
        }
        for value in &self.registry {
            platform::validate_registry_change(value, &self.app)?;
        }
        Ok(())
    }
    pub fn ensure_directory(
        &mut self,
        state: &Path,
        paths: &platform::ContextPaths,
        location: &FileLocation,
    ) -> Result<()> {
        let destination = location.resolve(paths)?;
        if destination.exists() {
            if !destination.is_dir() {
                bail!("Expected a directory: {}", destination.display());
            }
            return Ok(());
        }
        if let Some(parent) = location.parent() {
            self.ensure_directory(state, paths, &parent)?;
        } else {
            let base = destination.parent().context("Directory has no parent")?;
            reject_symlink_ancestors(base)?;
            fs::create_dir_all(base)?;
        }
        if !self.directories.contains(location) {
            self.directories.push(location.clone());
            self.save(state)?;
        }
        fs::create_dir(&destination)
            .with_context(|| format!("Create directory {}", destination.display()))?;
        Ok(())
    }
    pub fn write_file(
        &mut self,
        state: &Path,
        paths: &platform::ContextPaths,
        location: FileLocation,
        bytes: &[u8],
        executable: bool,
        retain: bool,
    ) -> Result<()> {
        let destination = location.resolve_file(paths)?;
        if self.files.iter().any(|f| f.location == location) {
            bail!(
                "Duplicate destination in transaction: {}",
                destination.display()
            );
        }
        if let Some(parent) = location.parent() {
            self.ensure_directory(state, paths, &parent)?;
        }
        let parent = destination
            .parent()
            .context("File has no parent directory")?;
        reject_symlink_ancestors(parent)?;
        fs::create_dir_all(parent)?;
        let original = if destination.exists() {
            if !destination.is_file() {
                bail!("Cannot replace non-file {}", destination.display());
            }
            let permissions = FilePermissions::capture(&destination)?;
            if permissions.readonly {
                bail!("Cannot replace read-only file {}", destination.display());
            }
            let bytes = bounded_read(&destination)?;
            let backup_name = format!("{:08}.bin", self.files.len());
            let backup_dir = state.join("backups");
            reject_symlink_ancestors(&backup_dir)?;
            fs::create_dir_all(&backup_dir)?;
            let backup = backup_path(state, &backup_name)?;
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&backup)?;
            output.write_all(&bytes)?;
            output.sync_all()?;
            sync_directory(&backup_dir)?;
            Some(OriginalFile {
                backup: backup_name,
                sha256: sha256(&bytes),
                permissions,
            })
        } else {
            None
        };
        let expected_original = original.as_ref().map(|o| o.sha256.clone());
        self.files.push(FileChange {
            location,
            installed_sha256: sha256(bytes),
            original,
            retain_on_uninstall: retain,
            undo_done: false,
            retained_backup: false,
        });
        self.save(state)?;
        reject_symlink_ancestors(&destination)?;
        if expected_original.is_some() {
            if !destination.is_file()
                || Some(sha256(&bounded_read(&destination)?)) != expected_original
            {
                bail!(
                    "File changed while preparing installation: {}",
                    destination.display()
                );
            }
        } else if destination.exists() {
            bail!(
                "File appeared while preparing installation: {}",
                destination.display()
            );
        }
        atomic_write(&destination, bytes, &FilePermissions::payload(executable))?;
        Ok(())
    }
    pub fn write_registry(
        &mut self,
        state: &Path,
        entry: &RegistryEntry,
        paths: &platform::ContextPaths,
    ) -> Result<()> {
        let change = platform::prepare_registry_change(entry, paths)?;
        if self.registry.iter().any(|r| {
            r.root == change.root
                && r.subkey.eq_ignore_ascii_case(&change.subkey)
                && r.name.eq_ignore_ascii_case(&change.name)
        }) {
            bail!(
                "Duplicate registry value in transaction: {}\\{} / {}",
                change.root,
                change.subkey,
                change.name
            );
        }
        self.registry.push(change.clone());
        self.save(state)?;
        if platform::get_registry(&change.root, &change.subkey, &change.name)? != change.previous {
            bail!("Registry value changed while preparing installation");
        }
        platform::set_registry(
            &change.root,
            &change.subkey,
            &change.name,
            &change.installed,
        )?;
        Ok(())
    }
    pub fn undo(
        &mut self,
        state: &Path,
        paths: &platform::ContextPaths,
        rollback: bool,
        report: &mut UninstallReport,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<()> {
        self.validate(state, paths)?;
        for index in (0..self.registry.len()).rev() {
            let record = &mut self.registry[index];
            if record.undo_done {
                report.metadata_retained |= record.retained_backup;
                continue;
            }
            if !rollback && !record.remove_on_uninstall {
                record.undo_done = true;
                self.save(state)?;
                continue;
            }
            let current = platform::get_registry(&record.root, &record.subkey, &record.name)?;
            let label = format!("{}\\{} / {}", record.root, record.subkey, record.name);
            if current == Some(record.installed.clone()) || current.is_none() {
                if let Some(previous) = &record.previous {
                    platform::set_registry(&record.root, &record.subkey, &record.name, previous)?;
                    report.restored.push(label);
                } else {
                    platform::delete_registry_value(&record.root, &record.subkey, &record.name)?;
                    report.removed.push(label);
                }
            } else if current == record.previous { /* already restored before crash */
            } else {
                report.preserved.push(label);
                record.retained_backup = record.previous.is_some();
                report.metadata_retained |= record.retained_backup;
            }
            for key in record.created_keys.iter().rev() {
                platform::delete_empty_registry_key(&record.root, key)?;
            }
            record.undo_done = true;
            self.save(state)?;
        }
        let total = self.files.len() as u64;
        for index in (0..self.files.len()).rev() {
            let record = &mut self.files[index];
            let destination = record.location.resolve_file(paths)?;
            let label = destination.display().to_string();
            progress(Progress {
                stage: if rollback { "rollback" } else { "uninstall" }.into(),
                item: label.clone(),
                completed: total - index as u64 - 1,
                total,
            });
            if record.undo_done {
                report.metadata_retained |= record.retained_backup;
                if record.retained_backup {
                    report.preserved.push(label);
                }
                continue;
            }
            if !rollback && record.retain_on_uninstall {
                report.preserved.push(label);
                record.retained_backup = record.original.is_some();
                report.metadata_retained |= record.retained_backup;
                record.undo_done = true;
                self.save(state)?;
                continue;
            }
            let current_hash = if destination.exists() {
                if !destination.is_file() {
                    None
                } else {
                    Some(sha256(&bounded_read(&destination)?))
                }
            } else {
                None
            };
            let exists = destination.exists();
            let matches_installed = current_hash.as_ref() == Some(&record.installed_sha256);
            let matches_original = record
                .original
                .as_ref()
                .is_some_and(|original| current_hash.as_ref() == Some(&original.sha256));
            if matches_original {
                // Equal content does not imply equal metadata. Installation may
                // have replaced a 0600 file with identical bytes at mode 0644.
                // Also completes a recovery that crashed after restoring bytes.
                if let Some(original) = &record.original {
                    original.permissions.apply(&destination)?;
                }
            } else if !exists || matches_installed {
                if let Some(original) = &record.original {
                    let bytes = bounded_read(&backup_path(state, &original.backup)?)?;
                    if sha256(&bytes) != original.sha256 {
                        bail!("Original backup is corrupt for {label}");
                    }
                    reject_symlink_ancestors(&destination)?;
                    atomic_write(&destination, &bytes, &original.permissions)?;
                    report.restored.push(label);
                } else if exists {
                    fs::remove_file(&destination)
                        .with_context(|| format!("Remove installed file {label}"))?;
                    report.removed.push(label);
                }
            } else {
                report.preserved.push(label);
                record.retained_backup = record.original.is_some();
                report.metadata_retained |= record.retained_backup;
            }
            record.undo_done = true;
            self.save(state)?;
        }
        for location in self.directories.iter().rev() {
            let path = location.resolve(paths)?;
            match fs::remove_dir(&path) {
                Ok(()) => (),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(e) => report.warnings.push(format!(
                    "Could not remove directory {}: {e}",
                    path.display()
                )),
            }
        }
        if report.metadata_retained {
            report.warnings.push(format!("Original backups were retained in {} because installed items were modified or marked to keep", state.display()));
        }
        progress(Progress {
            stage: "complete".into(),
            item: self.app.name.clone(),
            completed: total,
            total,
        });
        Ok(())
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn backup_path(state: &Path, name: &str) -> Result<PathBuf> {
    if name.len() != 12
        || !name.ends_with(".bin")
        || !name.as_bytes()[..8].iter().all(u8::is_ascii_digit)
    {
        bail!("Invalid backup name in journal");
    }
    let path = state.join("backups").join(name);
    reject_symlink_ancestors(&path)?;
    Ok(path)
}
fn bounded_read(path: &Path) -> Result<Vec<u8>> {
    reject_symlink_ancestors(path)?;
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() || metadata.len() > MAX_PACKAGE_BYTES {
        bail!("Not a supported regular file: {}", path.display());
    }
    Ok(fs::read(path)?)
}
fn atomic_write(path: &Path, bytes: &[u8], permissions: &FilePermissions) -> Result<()> {
    reject_symlink_ancestors(path)?;
    let parent = path.parent().context("Cannot write filesystem root")?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".ironsetup-write-")
        .tempfile_in(parent)?;
    temporary.write_all(bytes)?;
    permissions.apply(temporary.path())?;
    temporary.as_file().sync_all()?;
    reject_symlink_ancestors(path)?;
    temporary.persist(path).map_err(|e| e.error)?;
    sync_directory(parent)?;
    Ok(())
}
fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub(crate) struct LockGuard {
    _file: File,
}
impl LockGuard {
    pub fn acquire(state: &Path) -> Result<Self> {
        reject_symlink_ancestors(state)?;
        let path = state.join("lock");
        reject_symlink_ancestors(&path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        let file = options
            .open(&path)
            .context("Installation is busy or its metadata directory is unavailable")?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            unsafe extern "C" {
                fn flock(fd: i32, operation: i32) -> i32;
            }
            // LOCK_EX | LOCK_NB: advisory process lock, automatically released
            // by the OS on crash. No stale PID heuristics or lock-file deletion.
            if unsafe { flock(file.as_raw_fd(), 2 | 4) } != 0 {
                bail!("Another installer operation is already using this directory");
            }
        }
        Ok(Self { _file: file })
    }
}

pub(crate) fn cleanup_metadata(state: &Path) -> Result<()> {
    reject_symlink_ancestors(state)?;
    let backup_dir = state.join("backups");
    if backup_dir.exists() {
        reject_symlink_ancestors(&backup_dir)?;
        for entry in fs::read_dir(&backup_dir)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("Unknown backup filename"))?;
            let path = backup_path(state, &name)?;
            if !entry.file_type()?.is_file() {
                bail!("Refusing to remove unknown backup content");
            }
            fs::remove_file(path)?;
        }
        fs::remove_dir(backup_dir)?;
    }
    for name in ["journal.json", "lock"] {
        let path = state.join(name);
        reject_symlink_ancestors(&path)?;
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    // No recursive delete: an unexpected user file is preserved and reported.
    fs::remove_dir(state).context("Metadata contains unexpected files; they were preserved")?;
    Ok(())
}
