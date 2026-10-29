//! Transactional installer runtime. The installation journal is written before
//! each mutation, and originals are retained until uninstallation succeeds.
mod journal;
mod platform;
mod selection;

use anyhow::{bail, Context, Result};
use ironsetup_core::{
    evaluate_condition, sha256, validate_relative_path, AppInfo, Conditions, Manifest, Package,
    Progress, FORMAT_VERSION, MAX_FILES, MAX_PACKAGE_BYTES, STATE_DIR,
};
use journal::{FileLocation, Journal, JournalStatus, LockGuard};
pub use selection::default_selections;
use serde::Serialize;
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug)]
pub struct InstallOptions {
    pub target_dir: PathBuf,
    pub components: Option<BTreeSet<String>>,
    pub tasks: Option<BTreeSet<String>>,
    pub setup_type: Option<String>,
    pub no_run: bool,
    pub silent: bool,
    pub uninstaller: Option<Vec<u8>>,
}
impl InstallOptions {
    pub fn new(target_dir: PathBuf) -> Self {
        Self {
            target_dir,
            components: None,
            tasks: None,
            setup_type: None,
            no_run: false,
            silent: false,
            uninstaller: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct InstallReport {
    pub target_dir: PathBuf,
    pub installed_files: Vec<String>,
    pub skipped_files: Vec<String>,
    pub selected_components: BTreeSet<String>,
    pub selected_tasks: BTreeSet<String>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Default, Serialize)]
pub struct UninstallReport {
    pub target_dir: PathBuf,
    pub removed: Vec<String>,
    pub restored: Vec<String>,
    pub preserved: Vec<String>,
    pub warnings: Vec<String>,
    pub metadata_retained: bool,
}
#[derive(Debug, Default, Serialize)]
pub struct VerifyReport {
    pub target_dir: PathBuf,
    pub verified: Vec<String>,
    pub missing: Vec<String>,
    pub modified: Vec<String>,
    pub warnings: Vec<String>,
    pub intact: bool,
}

pub fn default_target_dir(manifest: &Manifest) -> Result<PathBuf> {
    let mut context = platform::ContextPaths::new(&manifest.app, None)?;
    let expanded = context.expand(&manifest.app.default_dir)?;
    let target = normalize_target(Path::new(&expanded.replace('\\', "/")))?;
    context.app = target.clone();
    Ok(target)
}

fn normalize_target(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!(
            "Installation directory must be an absolute path: {}",
            path.display()
        );
    }
    if path
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        bail!(
            "Installation directory must not contain '.' or '..': {}",
            path.display()
        );
    }
    if path.parent().is_none() || path.file_name().is_none() {
        bail!("Installing at a filesystem root is not allowed");
    }
    ironsetup_core::reject_symlink_ancestors(path)?;
    if path.exists() {
        if !path.is_dir() {
            bail!("Installation target is not a directory: {}", path.display());
        }
        Ok(fs::canonicalize(path)?)
    } else {
        Ok(path.to_path_buf())
    }
}

fn enabled(
    c: &Conditions,
    components: &BTreeSet<String>,
    tasks: &BTreeSet<String>,
) -> Result<bool> {
    Ok(evaluate_condition(&c.components, components)? && evaluate_condition(&c.tasks, tasks)?)
}

fn validate_package(package: &Package) -> Result<()> {
    let m = &package.manifest;
    if m.format_version != FORMAT_VERSION {
        bail!("Unsupported package format {}", m.format_version);
    }
    if m.files.len() > MAX_FILES || package.payloads.len() != m.files.len() {
        bail!("Payload count does not match manifest");
    }
    if m.app.id.trim().is_empty() || m.app.name.trim().is_empty() || m.app.version.trim().is_empty()
    {
        bail!("Application identity is incomplete");
    }
    if !matches!(m.app.privileges.as_str(), "lowest" | "admin") {
        bail!("Unsupported privilege mode");
    }
    let mut total = 0u64;
    let mut paths = HashSet::new();
    for (entry, data) in m.files.iter().zip(&package.payloads) {
        validate_relative_path(&entry.path)?;
        if !paths.insert(entry.path.replace('\\', "/").to_lowercase()) {
            bail!("Duplicate target path: {}", entry.path);
        }
        total = total
            .checked_add(entry.size)
            .context("Payload size overflow")?;
        if total > MAX_PACKAGE_BYTES {
            bail!("Package exceeds supported uncompressed size limit");
        }
        if entry.size != data.len() as u64 || sha256(data) != entry.sha256 {
            bail!("Payload integrity failure: {}", entry.path);
        }
        for flag in &entry.flags {
            if !matches!(
                flag.as_str(),
                "ignoreversion"
                    | "onlyifdoesntexist"
                    | "uninsneveruninstall"
                    | "recursesubdirs"
                    | "createallsubdirs"
                    | "skipifsourcedoesntexist"
            ) {
                bail!("Unsupported file flag: {flag}");
            }
        }
        if !entry
            .flags
            .iter()
            .any(|f| matches!(f.as_str(), "ignoreversion" | "onlyifdoesntexist"))
        {
            bail!("Files must explicitly use ignoreversion or onlyifdoesntexist; PE version comparison is unsupported");
        }
    }
    for dir in &m.dirs {
        validate_relative_path(&dir.path)?;
    }
    for entry in &m.registry {
        platform::validate_registry_entry(entry, &m.app)?;
    }
    for entry in &m.run {
        for flag in &entry.flags {
            if !matches!(
                flag.as_str(),
                "skipifsilent" | "nowait" | "runhidden" | "waituntilterminated"
            ) {
                bail!("Unsupported Run flag: {flag}");
            }
        }
        if entry.flags.iter().any(|f| f == "nowait")
            && entry.flags.iter().any(|f| f == "waituntilterminated")
        {
            bail!("Conflicting Run wait flags");
        }
        app_relative(&entry.filename)?;
        if !entry.working_dir.is_empty() {
            app_relative(&entry.working_dir)?;
        }
    }
    Ok(())
}

/// Install into one application directory. Existing IronSetup installations are
/// refused rather than performing an unsafe partial upgrade.
pub fn install(
    package: &Package,
    options: &InstallOptions,
    progress: &mut dyn FnMut(Progress),
) -> Result<InstallReport> {
    validate_package(package)?;
    let m = &package.manifest;
    let target = normalize_target(&options.target_dir)?;
    let paths = platform::ContextPaths::new(&m.app, Some(target.clone()))?;
    let (components, tasks) = selection::resolve_selections(m, options)?;
    platform::check_privileges(&m.app)?;
    let active_icons = m
        .icons
        .iter()
        .filter_map(
            |entry| match enabled(&entry.conditions, &components, &tasks) {
                Ok(true) => Some(Ok(entry)),
                Ok(false) => None,
                Err(e) => Some(Err(e)),
            },
        )
        .collect::<Result<Vec<_>>>()?;
    let active_registry = m
        .registry
        .iter()
        .filter_map(
            |entry| match enabled(&entry.conditions, &components, &tasks) {
                Ok(true) => Some(Ok(entry)),
                Ok(false) => None,
                Err(e) => Some(Err(e)),
            },
        )
        .collect::<Result<Vec<_>>>()?;
    #[cfg(not(windows))]
    if !active_icons.is_empty() || !active_registry.is_empty() {
        bail!("This package contains Windows shortcuts or registry actions; install it on Windows");
    }
    for icon in &active_icons {
        let location = paths.shortcut_location(&icon.name)?;
        location.resolve_file(&paths)?;
        paths.app_filename(&icon.filename, true)?;
        if icon.filename.eq_ignore_ascii_case("{uninstallexe}")
            && (!m.app.uninstallable || options.uninstaller.is_none())
        {
            bail!("Uninstall shortcut requires an available uninstaller");
        }
        if !icon.working_dir.is_empty() {
            paths.app_filename(&icon.working_dir, false)?;
        }
        if !icon.icon_filename.is_empty() {
            paths.app_filename(&icon.icon_filename, false)?;
        }
        paths.expand(&icon.parameters)?;
    }
    for entry in &active_registry {
        paths.expand(&entry.value_data)?;
    }
    // Preflight all destinations before creating installation metadata.
    let mut selected = Vec::new();
    let mut skipped = Vec::new();
    for (index, entry) in m.files.iter().enumerate() {
        if !enabled(&entry.conditions, &components, &tasks)? {
            skipped.push(entry.path.clone());
            continue;
        }
        let dest = ironsetup_core::checked_join(&target, &entry.path)?;
        if dest.exists() {
            if !dest.is_file() {
                bail!("Destination is not a regular file: {}", dest.display());
            }
            if entry.flags.iter().any(|f| f == "onlyifdoesntexist") {
                skipped.push(entry.path.clone());
                continue;
            }
            if fs::metadata(&dest)?.permissions().readonly() {
                bail!("Refusing to overwrite a read-only file: {}", dest.display());
            }
        }
        selected.push(index);
    }
    for dir in &m.dirs {
        if enabled(&dir.conditions, &components, &tasks)? {
            ironsetup_core::checked_join(&target, &dir.path)?;
        }
    }
    for run in &m.run {
        if enabled(&run.conditions, &components, &tasks)?
            && !options.no_run
            && !(options.silent && run.flags.iter().any(|f| f == "skipifsilent"))
        {
            #[cfg(not(windows))]
            if run.flags.iter().any(|f| f == "runhidden") {
                bail!("Run flag runhidden requires Windows");
            }
            paths.app_filename(&run.filename, false)?;
            paths.expand(&run.parameters)?;
            if !run.working_dir.is_empty() {
                paths.app_filename(&run.working_dir, false)?;
            }
        }
    }
    if target.join(STATE_DIR).exists() {
        bail!("An IronSetup installation or pending transaction already exists at {}. Uninstall it first, or recover an interrupted transaction", target.display());
    }
    let target_created = !target.exists();
    fs::create_dir_all(&target)
        .with_context(|| format!("Create installation directory {}", target.display()))?;
    let target = normalize_target(&target)?;
    let paths = platform::ContextPaths::new(&m.app, Some(target.clone()))?;
    let state_dir = target.join(STATE_DIR);
    fs::create_dir(&state_dir).context("Acquire new installation transaction directory")?;
    let lock = match LockGuard::acquire(&state_dir) {
        Ok(lock) => lock,
        Err(error) => {
            let _ = journal::cleanup_metadata(&state_dir);
            if target_created {
                let _ = fs::remove_dir(&target);
            }
            return Err(error);
        }
    };
    let mut journal = Journal::new(m.app.clone(), target.clone(), target_created);
    if let Err(error) = journal.save(&state_dir) {
        drop(lock);
        let _ = journal::cleanup_metadata(&state_dir);
        if target_created {
            let _ = fs::remove_dir(&target);
        }
        return Err(error);
    }
    let mut report = InstallReport {
        target_dir: target.clone(),
        installed_files: Vec::new(),
        skipped_files: skipped,
        selected_components: components.clone(),
        selected_tasks: tasks.clone(),
        warnings: Vec::new(),
    };
    let mutation = (|| -> Result<()> {
        for dir in &m.dirs {
            if enabled(&dir.conditions, &components, &tasks)? {
                journal.ensure_directory(
                    &state_dir,
                    &paths,
                    &FileLocation::app(dir.path.clone()),
                )?;
            }
        }
        for (n, index) in selected.iter().enumerate() {
            let entry = &m.files[*index];
            progress(Progress {
                stage: "install".into(),
                item: entry.path.clone(),
                completed: n as u64,
                total: selected.len() as u64,
            });
            journal.write_file(
                &state_dir,
                &paths,
                FileLocation::app(entry.path.clone()),
                &package.payloads[*index],
                entry.executable,
                entry.flags.iter().any(|f| f == "uninsneveruninstall"),
            )?;
            report.installed_files.push(entry.path.clone());
        }
        if m.app.uninstallable {
            if let Some(bytes) = &options.uninstaller {
                journal.write_file(
                    &state_dir,
                    &paths,
                    FileLocation::uninstaller(),
                    bytes,
                    true,
                    false,
                )?;
            } else {
                report.warnings.push("No standalone uninstaller was supplied; use ironsetup uninstall --dir <directory>".into());
            }
        }
        for icon in active_icons {
            let location = paths.shortcut_location(&icon.name)?;
            let bytes = platform::shortcut_bytes(icon, &paths, &state_dir)?;
            journal.write_file(&state_dir, &paths, location, &bytes, false, false)?;
        }
        for entry in active_registry {
            journal.write_registry(&state_dir, entry, &paths)?;
        }
        #[cfg(windows)]
        if m.app.uninstallable && options.uninstaller.is_some() {
            for entry in platform::uninstall_registration(&m.app, &target, package)? {
                journal.write_registry(&state_dir, &entry, &paths)?;
            }
        }
        journal.status = JournalStatus::Installed;
        journal.save(&state_dir)?;
        Ok(())
    })();
    if let Err(error) = mutation {
        journal.status = JournalStatus::RollingBack;
        let _ = journal.save(&state_dir);
        let mut rollback = UninstallReport {
            target_dir: target.clone(),
            ..Default::default()
        };
        let restored = journal.undo(&state_dir, &paths, true, &mut rollback, progress);
        drop(lock);
        if restored.is_ok() && !rollback.metadata_retained {
            journal::cleanup_metadata(&state_dir)?;
            if target_created {
                let _ = fs::remove_dir(&target);
            }
            return Err(
                error.context("Installation failed; installer-managed changes were rolled back")
            );
        }
        return Err(error.context(format!("Installation failed; recovery metadata retained at {}. Rollback status: {:?}; preserved: {:?}", state_dir.display(), restored.err(), rollback.preserved)));
    }
    drop(lock);
    // Run actions occur after the transaction commits. Their external effects
    // cannot honestly be rolled back, and failure leaves the installed app usable.
    if !options.no_run {
        for entry in &m.run {
            if !enabled(&entry.conditions, &components, &tasks)?
                || options.silent && entry.flags.iter().any(|f| f == "skipifsilent")
            {
                continue;
            }
            match platform::run_program(entry, &paths) {
                Ok(Some(code)) if code != 0 => report.warnings.push(format!(
                    "Post-install program '{}' exited with {code}; installation remains committed",
                    entry.filename
                )),
                Err(error) => report.warnings.push(format!(
                    "Could not run '{}': {error:#}; installation remains committed",
                    entry.filename
                )),
                _ => (),
            }
            if entry.flags.iter().any(|f| f == "nowait") {
                report.warnings.push(format!(
                    "Started '{}' without waiting; its eventual exit status is unknown",
                    entry.filename
                ));
            }
        }
    }
    progress(Progress {
        stage: "complete".into(),
        item: m.app.name.clone(),
        completed: selected.len() as u64,
        total: selected.len() as u64,
    });
    Ok(report)
}

pub fn uninstall(target_dir: &Path, progress: &mut dyn FnMut(Progress)) -> Result<UninstallReport> {
    undo_installation(target_dir, false, progress)
}

/// Restore an interrupted transaction. Never treats an installed application as
/// an incomplete transaction; use uninstall for a committed installation.
pub fn recover(target_dir: &Path, progress: &mut dyn FnMut(Progress)) -> Result<UninstallReport> {
    undo_installation(target_dir, true, progress)
}

fn undo_installation(
    target_dir: &Path,
    recovery: bool,
    progress: &mut dyn FnMut(Progress),
) -> Result<UninstallReport> {
    let target = normalize_target(target_dir)?;
    let state_dir = target.join(STATE_DIR);
    ironsetup_core::reject_symlink_ancestors(&state_dir)?;
    let lock = LockGuard::acquire(&state_dir)?;
    let mut journal = Journal::load(&state_dir, &target)?;
    platform::check_privileges(&journal.app)?;
    if recovery {
        if journal.status == JournalStatus::Installed {
            bail!("Installation is committed; use uninstall instead of recover");
        }
    } else {
        if !journal.app.uninstallable {
            bail!("This application was compiled with Uninstallable=no");
        }
        if !matches!(
            journal.status,
            JournalStatus::Installed | JournalStatus::Uninstalling
        ) {
            bail!("Installation was interrupted; use recover before installing again");
        }
    }
    let rollback = recovery && journal.status != JournalStatus::Uninstalling;
    let paths = platform::ContextPaths::new(&journal.app, Some(target.clone()))?;
    journal.validate(&state_dir, &paths)?;
    journal.status = if rollback {
        JournalStatus::RollingBack
    } else {
        JournalStatus::Uninstalling
    };
    journal.save(&state_dir)?;
    let mut report = UninstallReport {
        target_dir: target.clone(),
        ..Default::default()
    };
    journal.undo(&state_dir, &paths, rollback, &mut report, progress)?;
    drop(lock);
    if !report.metadata_retained {
        journal::cleanup_metadata(&state_dir)?;
        if journal.target_created {
            let _ = fs::remove_dir(&target);
        }
    }
    Ok(report)
}

pub fn verify_install(target_dir: &Path) -> Result<VerifyReport> {
    let target = normalize_target(target_dir)?;
    let state_dir = target.join(STATE_DIR);
    ironsetup_core::reject_symlink_ancestors(&state_dir)?;
    let _lock = LockGuard::acquire(&state_dir)?;
    let journal = Journal::load(&state_dir, &target)?;
    if journal.status != JournalStatus::Installed {
        bail!("Installation is not committed ({:?})", journal.status);
    }
    let paths = platform::ContextPaths::new(&journal.app, Some(target.clone()))?;
    journal.validate(&state_dir, &paths)?;
    let mut report = VerifyReport {
        target_dir: target,
        intact: true,
        ..Default::default()
    };
    for file in &journal.files {
        let path = file.location.resolve_file(&paths)?;
        if !path.exists() {
            report.missing.push(path.display().to_string());
        } else if !path.is_file() || sha256(&fs::read(&path)?) != file.installed_sha256 {
            report.modified.push(path.display().to_string());
        } else {
            report.verified.push(path.display().to_string());
        }
    }
    report.intact = report.missing.is_empty() && report.modified.is_empty();
    if !journal.registry.is_empty() {
        report.warnings.push("Verification checks installed files and shortcuts; registry values are not included in the intact result".into());
    }
    Ok(report)
}

/// Read the installed application's identity and declared privilege mode.
/// This performs the same path, journal and backup validation as uninstall but
/// does not create a lock file, mutate state, or request elevation itself.
pub fn installed_app_info(target_dir: &Path) -> Result<AppInfo> {
    let target = normalize_target(target_dir)?;
    let state_dir = target.join(STATE_DIR);
    ironsetup_core::reject_symlink_ancestors(&state_dir)?;
    let journal = Journal::load(&state_dir, &target)?;
    let paths = platform::ContextPaths::new(&journal.app, Some(target))?;
    journal.validate(&state_dir, &paths)?;
    Ok(journal.app)
}

pub(crate) fn app_relative(value: &str) -> Result<Option<String>> {
    if value.eq_ignore_ascii_case("{app}") {
        return Ok(None);
    }
    if !value
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("{app}"))
    {
        bail!("Only paths rooted at {{app}} are supported here");
    }
    let tail = value[5..]
        .strip_prefix(['\\', '/'])
        .context("Only paths rooted at {app} are supported here")?;
    validate_relative_path(tail)?;
    Ok(Some(tail.replace('\\', "/")))
}
