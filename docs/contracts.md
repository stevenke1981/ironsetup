# v0.1 crate contracts

`ironsetup-core::model` is owned by root. Request model changes rather than editing it in an agent worktree. `Package { manifest, payloads }` stores uncompressed file vectors aligned with `manifest.files`. Root implements bounded deterministic compressed encoding, embedded EXE trailer, digest validation, portable path guard, boolean condition evaluation.

## Compiler
Expose `CompileOptions` (`defines: BTreeMap<String,String>`, Default), `CompiledScript { package: Package, output_dir: PathBuf, output_basename: String, warnings: Vec<Diagnostic> }`, and `compile_script(path: &Path, options: &CompileOptions) -> anyhow::Result<CompiledScript>`. Expose `preprocess(path, options) -> Result<String>` if feasible.
Source-line-aware errors. Strict unknown sections/keys/flags. UTF-8 BOM allowed. `#define NAME "value"`, `{#NAME}`, `#include "relative.iss"`, bounded include cycles. Windows-style paths on Linux. Explicit [Files] under {app}, glob expansion + recursion/excludes; packages deterministic. Directives and flags are supported only if implemented by engine or UI. Required AppName/AppVersion/DefaultDirName; AppId default AppName; PrivilegesRequired default lowest (document difference). `[Code]`/Pascal rejected. Default compression zip, not LZMA2. Optional text LicenseFile/InfoBeforeFile/InfoAfterFile must be embedded. Compiler itself writes no output.

## Engine
Expose `InstallOptions { target_dir: PathBuf, components: Option<BTreeSet<String>>, tasks: Option<BTreeSet<String>>, setup_type: Option<String>, no_run: bool, silent: bool, uninstaller: Option<Vec<u8>> }`; constructor `InstallOptions::new(target_dir: PathBuf)`.
Expose `install(package: &Package, options: &InstallOptions, progress: &mut dyn FnMut(Progress)) -> Result<InstallReport>`, `uninstall(target_dir: &Path, progress: &mut dyn FnMut(Progress)) -> Result<UninstallReport>`, `verify_install(target_dir: &Path) -> Result<VerifyReport>`, `default_target_dir(manifest: &Manifest) -> Result<PathBuf>`, `default_selections(manifest: &Manifest, setup_type: Option<&str>) -> Result<(BTreeSet<String>, BTreeSet<String>)>`.
Reports derive Debug + Serialize, fields are at engine discretion. Installer rollback journaling, safe path/reparse handling, original backups; modification-aware uninstall preserving unowned/user-modified files. Single-directory install includes `.ironsetup` journal and `unins000.exe`. Prefer support upgrades with safe original-backup preservation; otherwise reject repeat install clearly. Win32 registry and .lnk shortcuts use native APIs. On non-Windows reject Windows-only manifest actions BEFORE mutations. `[Run]` no shell concatenation, explicit allowed flags; failed arbitrary external programs cannot be fully rolled back and must be reported. Central selections reused by CLI and GUI. Uninstall registry entry on Windows. Never recursively delete the entire app directory.

## UI
UI depends on core ONLY; no circular dependencies. Native Windows controls with `windows-sys`, cfg fallback errors on non-Windows. Expose:

```
pub struct WizardSelection { pub target_dir: PathBuf, pub components: BTreeSet<String>, pub tasks: BTreeSet<String> }
pub fn installer_wizard(manifest: &Manifest, default_dir: &Path,
  perform: Box<dyn FnOnce(WizardSelection, Box<dyn Fn(Progress) + Send>) -> Result<String> + Send>) -> Result<()>;
pub fn compiler_studio(initial_script: Option<PathBuf>,
  compile: Box<dyn Fn(PathBuf) -> Result<String> + Send + Sync>) -> Result<()>;
pub fn confirm_uninstall(app_name: &str, target: &Path) -> Result<bool>;
pub fn show_error(message: &str);
```

Installer: welcome/license (accept required)/destination/features/tasks/ready/progress/completion; worker thread keeps GUI responsive, disable close/cancel during filesystem commit. Default selected choices align engine default_selections (root may extend function input if needed). Studio: native .iss editor with Open/Save, compile button + output log/status; build worker saves UTF-8 script and calls injected compiler callback. Native file dialogs. Helpful explicit non-Windows UI error. UI text en-US + Traditional Chinese if feasible, English acceptable for v0.1 with zh-TW docs. Actual Windows execution evidence is separate from compile success.
