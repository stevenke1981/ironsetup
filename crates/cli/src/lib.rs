use anyhow::{bail, ensure, Context, Result};
use ironsetup_compiler::{compile_script, CompileOptions};
use ironsetup_core::{Package, Progress};
use ironsetup_engine::InstallOptions;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

mod windows;

const HELP: &str = r#"IronSetup 0.1.0 — Rust installer compiler

  ironsetup compile script.iss [--output setup.exe] [--stub path] [-DNAME=value]
  ironsetup compile script.iss --package-only [--output app.ispkg]
  ironsetup check script.iss
  ironsetup preprocess script.iss [-DNAME=value]
  ironsetup inspect setup.exe|app.ispkg
  ironsetup install app.ispkg|setup.exe --dir PATH [--components core,docs]
                    [--tasks desktopicon] [--type full] [--no-run] [--log PATH]
  ironsetup verify --dir PATH
  ironsetup uninstall --dir PATH
  ironsetup recover --dir PATH
  ironsetup studio [script.iss]

ISCC-style compilation: ironsetup /Q /Ooutput /Fname /DKey=value script.iss
Installer aliases: /VERYSILENT /DIR=path /COMPONENTS=list /TASKS=list /TYPE=name
                   /LOG[=path] /NORESTART /SP- /SUPPRESSMSGBOXES
--silent and /SILENT both suppress the entire GUI in v0.1.
Use --help to read this text. Unsupported .iss features are errors.
"#;

#[derive(Default, Debug)]
struct BuildArgs {
    script: Option<PathBuf>,
    output: Option<PathBuf>,
    output_dir: Option<PathBuf>,
    output_base: Option<String>,
    stub: Option<PathBuf>,
    defines: BTreeMap<String, String>,
    package_only: bool,
    quiet: bool,
    json: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct BuildReport {
    pub output: PathBuf,
    pub app: String,
    pub version: String,
    pub files: usize,
    pub payload_bytes: u64,
    pub output_bytes: u64,
    pub warnings: Vec<ironsetup_core::Diagnostic>,
}

fn next_value(args: &[String], index: &mut usize, label: &str) -> Result<String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .with_context(|| format!("{label} requires a value"))
}
fn set_once<T>(slot: &mut Option<T>, value: T, label: &str) -> Result<()> {
    ensure!(slot.is_none(), "{label} was specified more than once");
    *slot = Some(value);
    Ok(())
}
fn parse_build(args: &[String]) -> Result<BuildArgs> {
    let mut b = BuildArgs::default();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let lower = a.to_ascii_lowercase();
        #[cfg(not(windows))]
        if Path::new(a).is_absolute() && lower.ends_with(".iss") {
            set_once(&mut b.script, PathBuf::from(a), "script")?;
            i += 1;
            continue;
        }
        match lower.as_str() {
            "--output" | "-o" => set_once(&mut b.output, next_value(args, &mut i, a)?.into(), a)?,
            "--output-dir" => set_once(&mut b.output_dir, next_value(args, &mut i, a)?.into(), a)?,
            "--output-filename" => set_once(&mut b.output_base, next_value(args, &mut i, a)?, a)?,
            "--stub" => set_once(&mut b.stub, next_value(args, &mut i, a)?.into(), a)?,
            "--package-only" => b.package_only = true,
            "--quiet" | "/q" | "-q" => b.quiet = true,
            "--json" => b.json = true,
            "--define" | "-d" => {
                let d = next_value(args, &mut i, a)?;
                add_define(&mut b.defines, &d)?;
            }
            _ if a.starts_with("--output=") => set_once(&mut b.output, a[9..].into(), "output")?,
            _ if a.starts_with("--stub=") => set_once(&mut b.stub, a[7..].into(), "stub")?,
            _ if a.starts_with("--define=") => add_define(&mut b.defines, &a[9..])?,
            _ if a.starts_with("-D") || a.starts_with("/D") || a.starts_with("/d") => {
                add_define(&mut b.defines, &a[2..])?
            }
            _ if a.starts_with("/O") || a.starts_with("/o") => {
                set_once(&mut b.output_dir, a[2..].into(), "output-dir")?
            }
            _ if a.starts_with("/F") || a.starts_with("/f") => {
                set_once(&mut b.output_base, a[2..].into(), "output-filename")?
            }
            _ if a.starts_with('-') => bail!("Unknown compiler option: {a}"),
            _ => set_once(&mut b.script, PathBuf::from(a), "script")?,
        }
        i += 1;
    }
    ensure!(b.script.is_some(), "A .iss script is required");
    Ok(b)
}
fn add_define(defines: &mut BTreeMap<String, String>, value: &str) -> Result<()> {
    let (name, value) = value.split_once('=').unwrap_or((value, "1"));
    ensure!(
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
        "Invalid define name: {name}"
    );
    ensure!(
        defines.insert(name.into(), value.into()).is_none(),
        "Duplicate define: {name}"
    );
    Ok(())
}
fn sibling_stub() -> Result<PathBuf> {
    let current = std::env::current_exe()?;
    let name = if cfg!(windows) {
        "ironsetup-stub.exe"
    } else {
        "ironsetup-stub"
    };
    let path = current.with_file_name(name);
    ensure!(path.is_file(),"Installer stub not found at {}. Build the full workspace or pass --stub PATH; use --package-only for an archive.",path.display());
    Ok(path)
}
fn build(b: &BuildArgs) -> Result<BuildReport> {
    let compiled = compile_script(
        b.script.as_ref().context("Missing script")?,
        &CompileOptions {
            defines: b.defines.clone(),
        },
    )?;
    ironsetup_core::validate_package(&compiled.package)?;
    let package_only = b.package_only
        || b.output
            .as_ref()
            .and_then(|p| p.extension())
            .is_some_and(|e| e.eq_ignore_ascii_case("ispkg"));
    let stub = if package_only {
        None
    } else {
        Some(match &b.stub {
            Some(s) => s.clone(),
            None => sibling_stub()?,
        })
    };
    let extension = if package_only {
        "ispkg"
    } else {
        let stub_bytes = ironsetup_core::stub_bytes(stub.as_ref().unwrap())?;
        if stub_bytes.starts_with(b"MZ") {
            "exe"
        } else if stub_bytes.starts_with(b"\x7fELF") {
            "run"
        } else {
            bail!("Stub must be a Windows PE or Linux ELF executable");
        }
    };
    let output = match &b.output {
        Some(p) => p.clone(),
        None => {
            let dir = b.output_dir.clone().unwrap_or(compiled.output_dir);
            let name = b
                .output_base
                .as_deref()
                .unwrap_or(&compiled.output_basename);
            ironsetup_core::validate_relative_path(name).context("Unsafe output filename")?;
            ensure!(
                !name.contains(['/', '\\']),
                "OutputBaseFilename must be a filename"
            );
            dir.join(format!("{name}.{extension}"))
        }
    };
    if package_only {
        ironsetup_core::write_package_file(&output, &compiled.package)?;
    } else {
        ironsetup_core::append_package(stub.as_ref().unwrap(), &compiled.package, &output)?;
    }
    Ok(BuildReport {
        output: std::path::absolute(&output)?,
        app: compiled.package.manifest.app.name,
        version: compiled.package.manifest.app.version,
        files: compiled.package.manifest.files.len(),
        payload_bytes: compiled.package.manifest.files.iter().map(|f| f.size).sum(),
        output_bytes: std::fs::metadata(&output)?.len(),
        warnings: compiled.warnings,
    })
}

#[derive(Default, Debug, Clone)]
struct SetupArgs {
    dir: Option<PathBuf>,
    components: Option<BTreeSet<String>>,
    tasks: Option<BTreeSet<String>>,
    setup_type: Option<String>,
    no_run: bool,
    silent: bool,
    log: Option<Option<PathBuf>>,
    uninstall: bool,
    parent_pid: Option<u32>,
    confirmed: bool,
}
fn names(value: &str) -> Result<BTreeSet<String>> {
    if value.is_empty() {
        return Ok(BTreeSet::new());
    }
    value
        .split(',')
        .map(|s| {
            let n = s.trim();
            ensure!(
                !n.is_empty()
                    && n.chars()
                        .all(|c| c.is_alphanumeric() || "_-.\\/".contains(c)),
                "Invalid selection: {n}; v0.1 accepts exact comma-separated names"
            );
            Ok(n.to_lowercase())
        })
        .collect()
}
fn parse_setup(args: &[String]) -> Result<SetupArgs> {
    let mut s = SetupArgs::default();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let (key, inline) = a.split_once('=').map_or((a.as_str(), None), |(k, v)| {
            (k, Some(v.trim_matches('"').to_string()))
        });
        let key = key.trim_start_matches(['-', '/']).to_ascii_lowercase();
        let mut val = || -> Result<String> {
            match &inline {
                Some(v) => Ok(v.clone()),
                None => next_value(args, &mut i, a),
            }
        };
        match key.as_str() {
            "dir" | "target-dir" => set_once(&mut s.dir, PathBuf::from(val()?), "dir")?,
            "components" => set_once(&mut s.components, names(&val()?)?, "components")?,
            "tasks" => set_once(&mut s.tasks, names(&val()?)?, "tasks")?,
            "type" => set_once(&mut s.setup_type, val()?.to_lowercase(), "type")?,
            "silent" | "verysilent" | "very-silent" => {
                ensure!(inline.is_none(), "{a} does not accept a value");
                s.silent = true;
            }
            "no-run" => {
                ensure!(inline.is_none(), "{a} does not accept a value");
                s.no_run = true;
            }
            "uninstall" => {
                ensure!(inline.is_none(), "{a} does not accept a value");
                s.uninstall = true;
            }
            "confirmed" => {
                ensure!(inline.is_none(), "{a} does not accept a value");
                s.confirmed = true;
            }
            "parent-pid" => s.parent_pid = Some(val()?.parse().context("Invalid parent PID")?),
            "log" => {
                ensure!(s.log.is_none(), "Log specified more than once");
                s.log = Some(if let Some(v) = inline {
                    Some(PathBuf::from(v))
                } else if a.starts_with("--")
                    && args.get(i + 1).is_some_and(|n| !n.starts_with('-'))
                {
                    Some(PathBuf::from(next_value(args, &mut i, a)?))
                } else {
                    None
                });
            }
            "sp-" | "norestart" | "suppressmsgboxes" => {
                ensure!(inline.is_none(), "{a} does not accept a value")
            }
            _ => bail!("Unknown installer option: {a}"),
        }
        i += 1;
    }
    Ok(s)
}

#[derive(Clone)]
struct Log {
    file: Option<Arc<Mutex<File>>>,
}
impl Log {
    fn open(request: &Option<Option<PathBuf>>) -> Result<Self> {
        let Some(request) = request else {
            return Ok(Self { file: None });
        };
        let path = request.clone().unwrap_or_else(|| {
            std::env::temp_dir().join(format!(
                "IronSetup-{}-{}.log",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            ))
        });
        ironsetup_core::reject_symlink_ancestors(&path)?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| {
                format!(
                    "Cannot create log {} (existing logs are not overwritten)",
                    path.display()
                )
            })?;
        let result = Self {
            file: Some(Arc::new(Mutex::new(file))),
        };
        result.line("IronSetup 0.1.0");
        Ok(result)
    }
    fn line(&self, text: &str) {
        if let Some(file) = &self.file {
            if let Ok(mut file) = file.lock() {
                let _ = writeln!(file, "{text}");
                let _ = file.flush();
            }
        }
    }
    fn progress(&self, p: &Progress) {
        self.line(&format!(
            "{} {}/{} {}",
            p.stage, p.completed, p.total, p.item
        ));
    }
}
fn load_package(path: &Path) -> Result<Package> {
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("ispkg"))
    {
        return ironsetup_core::read_package_file(path);
    }
    if let Some(p) = ironsetup_core::read_embedded_package(path)? {
        return Ok(p);
    }
    ironsetup_core::read_package_file(path)
}
fn install_options(args: &SetupArgs, target: PathBuf, stub: &Path) -> Result<InstallOptions> {
    let mut options = InstallOptions::new(target);
    options.components = args.components.clone();
    options.tasks = args.tasks.clone();
    options.setup_type = args.setup_type.clone();
    options.no_run = args.no_run;
    options.silent = args.silent;
    options.uninstaller = Some(ironsetup_core::stub_bytes(stub)?);
    Ok(options)
}

fn perform_install(
    package: &Package,
    args: &SetupArgs,
    target: PathBuf,
    stub: &Path,
    log: &Log,
    progress: &mut dyn FnMut(Progress),
) -> Result<String> {
    ironsetup_core::validate_package(package)?;
    let options = install_options(args, target, stub)?;
    let result = ironsetup_engine::install(package, &options, &mut |p| {
        log.progress(&p);
        progress(p);
    });
    match result {
        Ok(report) => {
            let s = serde_json::to_string_pretty(&report)?;
            log.line(&s);
            Ok(s)
        }
        Err(e) => {
            log.line(&format!("ERROR: {e:#}"));
            Err(e)
        }
    }
}

pub fn cli_main(args: Vec<String>) -> Result<i32> {
    if args.is_empty() || matches!(args[0].as_str(), "--help" | "-h" | "/?" | "/HELP") {
        println!("{HELP}");
        return Ok(0);
    }
    if matches!(args[0].as_str(), "--version" | "-V") {
        println!("IronSetup {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }
    match args[0].as_str() {
        "compile" | "build" | "check" | "preprocess" => {
            let b = parse_build(&args[1..])?;
            if args[0] == "preprocess" {
                print!(
                    "{}",
                    ironsetup_compiler::preprocess(
                        b.script.as_ref().unwrap(),
                        &CompileOptions { defines: b.defines }
                    )?
                );
                return Ok(0);
            }
            if args[0] == "check" {
                let c = compile_script(
                    b.script.as_ref().unwrap(),
                    &CompileOptions { defines: b.defines },
                )?;
                ironsetup_core::validate_package(&c.package)?;
                println!(
                    "PASS: {} — {} files",
                    c.package.manifest.app.name,
                    c.package.manifest.files.len()
                );
                return Ok(0);
            }
            let report = build(&b)?;
            print_build(&b, &report)?;
        }
        "inspect" => {
            ensure!(args.len() == 2, "inspect requires exactly one package path");
            println!(
                "{}",
                serde_json::to_string_pretty(&load_package(Path::new(&args[1]))?.manifest)?
            );
        }
        "install" => {
            ensure!(args.len() >= 2, "install requires a package path");
            let p = load_package(Path::new(&args[1]))?;
            let mut s = parse_setup(&args[2..])?;
            s.silent = true;
            let dir = match &s.dir {
                Some(d) => std::path::absolute(d)?,
                None => ironsetup_engine::default_target_dir(&p.manifest)?,
            };
            let log = Log::open(&s.log)?;
            println!(
                "{}",
                perform_install(&p, &s, dir, &sibling_stub()?, &log, &mut |_| {})?
            );
        }
        "uninstall" | "verify" | "recover" => {
            let s = parse_setup(&args[1..])?;
            let dir = std::path::absolute(s.dir.context("--dir PATH is required")?)?;
            let log = Log::open(&s.log)?;
            let mut exit_code = 0;
            let report = match args[0].as_str() {
                "uninstall" => {
                    serde_json::to_string_pretty(&ironsetup_engine::uninstall(&dir, &mut |p| {
                        log.progress(&p)
                    })?)?
                }
                "recover" => {
                    serde_json::to_string_pretty(&ironsetup_engine::recover(&dir, &mut |p| {
                        log.progress(&p)
                    })?)?
                }
                _ => {
                    let r = ironsetup_engine::verify_install(&dir)?;
                    if !r.intact {
                        exit_code = 3;
                    }
                    serde_json::to_string_pretty(&r)?
                }
            };
            log.line(&report);
            println!("{report}");
            if exit_code != 0 {
                return Ok(exit_code);
            }
        }
        "studio" => {
            ensure!(args.len() <= 2, "studio accepts one script path");
            studio_main(args.get(1).map(PathBuf::from))?;
        }
        _ => {
            let b = parse_build(&args)?;
            let r = build(&b)?;
            print_build(&b, &r)?;
        }
    }
    Ok(0)
}
fn print_build(b: &BuildArgs, r: &BuildReport) -> Result<()> {
    if b.json {
        println!("{}", serde_json::to_string_pretty(r)?);
    } else if !b.quiet {
        println!(
            "Built {}\n{} {}: {} files, {} payload bytes, {} output bytes",
            r.output.display(),
            r.app,
            r.version,
            r.files,
            r.payload_bytes,
            r.output_bytes
        );
    }
    for w in &r.warnings {
        eprintln!("{}:{}: warning: {}", w.file, w.line, w.message);
    }
    Ok(())
}

pub fn studio_main(initial: Option<PathBuf>) -> Result<()> {
    ironsetup_ui::compiler_studio(
        initial,
        Box::new(|script| {
            let r = build(&BuildArgs {
                script: Some(script),
                ..Default::default()
            })?;
            Ok(format!(
                "Build succeeded\r\n{}\r\n{} files, {} bytes\r\n{}",
                r.output.display(),
                r.files,
                r.output_bytes,
                r.warnings
                    .iter()
                    .map(|w| format!("{}:{}: {}", w.file, w.line, w.message))
                    .collect::<Vec<_>>()
                    .join("\r\n")
            ))
        }),
    )
}

pub fn stub_main(args: Vec<String>) -> Result<i32> {
    if args
        .iter()
        .any(|a| matches!(a.to_ascii_lowercase().as_str(), "--help" | "/?" | "/help"))
    {
        ironsetup_ui::show_information(HELP);
        return Ok(0);
    }
    let s = parse_setup(&args)?;
    let exe = std::env::current_exe()?;
    if s.uninstall
        || exe
            .file_stem()
            .is_some_and(|n| n.eq_ignore_ascii_case("unins000"))
    {
        let dir = match &s.dir {
            Some(d) => std::path::absolute(d)?,
            None => exe
                .parent()
                .context("Missing uninstall directory")?
                .to_path_buf(),
        };
        let installed_app = ironsetup_engine::installed_app_info(&dir)?;
        if installed_app.privileges == "admin" {
            if let Some(code) = windows::elevate_if_needed(&exe, &args)? {
                return Ok(code);
            }
        }
        if let Some(pid) = s.parent_pid {
            windows::wait_for_parent(pid)?;
        }
        if !s.silent && !s.confirmed && !ironsetup_ui::confirm_uninstall(&installed_app.name, &dir)?
        {
            return Ok(2);
        }
        if windows::relaunch_uninstaller_from_temp(&exe, &dir, &s)? {
            return Ok(0);
        }
        let log = Log::open(&s.log)?;
        let report = ironsetup_engine::uninstall(&dir, &mut |p| log.progress(&p));
        match report {
            Ok(r) => {
                log.line(&serde_json::to_string_pretty(&r)?);
                if !s.silent {
                    let mut message = format!("Uninstallation completed.\n\nRemoved: {} files\nRestored originals: {}\nPreserved: {}\n\n{}", r.removed.len(), r.restored.len(), r.preserved.len(), r.target_dir.display());
                    if !r.preserved.is_empty() {
                        message
                            .push_str(&format!("\n\nPreserved files:\n{}", r.preserved.join("\n")));
                    }
                    if !r.warnings.is_empty() {
                        message.push_str(&format!("\n\nNotes:\n{}", r.warnings.join("\n")));
                    }
                    if r.metadata_retained {
                        message.push_str("\n\nOriginal backups are retained in .ironsetup. See the log before removing this directory.");
                    }
                    ironsetup_ui::show_information(&message);
                }
                return Ok(0);
            }
            Err(e) => {
                log.line(&format!("ERROR: {e:#}"));
                return Err(e);
            }
        }
    }
    let package=ironsetup_core::read_embedded_package(&exe)?.context("This is an unbound installer stub. Compile a .iss script with ironsetup to create an installer.")?;
    if package.manifest.app.privileges == "admin" {
        if let Some(code) = windows::elevate_if_needed(&exe, &args)? {
            return Ok(code);
        }
    }
    let target = match &s.dir {
        Some(d) => std::path::absolute(d)?,
        None => ironsetup_engine::default_target_dir(&package.manifest)?,
    };
    let log = Log::open(&s.log)?;
    if s.silent {
        perform_install(&package, &s, target, &exe, &log, &mut |_| {})?;
        return Ok(0);
    }
    #[cfg(not(windows))]
    {
        bail!("This installer has no native GUI on this platform. Use --silent --dir PATH, or the ironsetup install command.");
    }
    #[cfg(windows)]
    {
        let (default_components, default_tasks) =
            ironsetup_engine::default_selections(&package.manifest, s.setup_type.as_deref())?;
        let selection = ironsetup_ui::WizardSelection {
            target_dir: target,
            components: s.components.clone().unwrap_or(default_components),
            tasks: s.tasks.clone().unwrap_or(default_tasks),
        };
        let manifest = package.manifest.clone();
        let outcome = ironsetup_ui::installer_wizard_with_selections(
            &manifest,
            selection,
            Box::new(move |selected, notify| {
                let mut chosen = s.clone();
                chosen.components = Some(selected.components);
                chosen.tasks = Some(selected.tasks);
                chosen.setup_type = None;
                perform_install(
                    &package,
                    &chosen,
                    selected.target_dir,
                    &exe,
                    &log,
                    &mut |p| notify(p),
                )
            }),
        );
        match outcome {
            Ok(()) => Ok(0),
            Err(error) if ironsetup_ui::is_cancelled(&error) => Ok(2),
            Err(error) if ironsetup_ui::is_operation_reported(&error) => Ok(1),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn parses_iscc_and_unix_paths() {
        let b = parse_build(&args(&[
            "/Q",
            "/Oout",
            "/Fdemo",
            "/DVersion=2.0",
            "/tmp/demo.iss",
        ]))
        .unwrap();
        assert!(b.quiet);
        assert_eq!(b.defines["Version"], "2.0");
        assert_eq!(b.script.unwrap(), PathBuf::from("/tmp/demo.iss"));
    }
    #[test]
    fn parses_setup_aliases_and_empty_tasks() {
        let s = parse_setup(&args(&[
            "/VERYSILENT",
            "/DIR=C:\\Demo App",
            "/COMPONENTS=Core,docs",
            "/TASKS=",
            "/LOG=C:\\demo.log",
        ]))
        .unwrap();
        assert!(s.silent);
        assert_eq!(s.components.unwrap().len(), 2);
        assert!(s.tasks.unwrap().is_empty());
        assert_eq!(s.dir.unwrap(), PathBuf::from("C:\\Demo App"));
    }
    #[test]
    fn invalid_cli_never_silently_ignored() {
        assert!(parse_setup(&args(&["/TYPO"])).is_err());
        assert!(parse_setup(&args(&["--dir"])).is_err());
        assert!(parse_setup(&args(&["--dir", "a", "--dir", "b"])).is_err());
        assert!(parse_build(&args(&["a.iss", "b.iss"])).is_err());
    }
}
