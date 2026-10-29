//! Native Windows front ends. Worker callbacks never receive window handles.
//! All controls, dialogs and message processing stay on the calling UI thread.

use anyhow::Result;
use ironsetup_core::{Manifest, Progress};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

mod flow;
#[cfg(windows)]
mod windows;

#[derive(Clone, Debug)]
pub struct WizardSelection {
    pub target_dir: PathBuf,
    pub components: BTreeSet<String>,
    pub tasks: BTreeSet<String>,
}

pub type InstallCallback =
    Box<dyn FnOnce(WizardSelection, Box<dyn Fn(Progress) + Send>) -> Result<String> + Send>;
pub type CompileCallback = Box<dyn Fn(PathBuf) -> Result<String> + Send + Sync>;

/// Expected wizard outcomes are distinguishable from window startup failures.
#[derive(Debug)]
pub enum WizardError {
    /// The user cancelled before the installer callback was started.
    Cancelled,
    /// The callback failed and its error was already shown on the result page.
    OperationFailed(String),
}

impl std::fmt::Display for WizardError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => {
                formatter.write_str("Installation cancelled before any changes were made")
            }
            Self::OperationFailed(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for WizardError {}

/// Hosts can return cancellation exit code 2 without showing an error dialog.
pub fn is_cancelled(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<WizardError>(),
        Some(WizardError::Cancelled)
    )
}

/// True only when the wizard has already displayed a failed operation's result.
pub fn is_operation_reported(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<WizardError>(),
        Some(WizardError::OperationFailed(_))
    )
}

/// Open the installer using the same documented default-selection rules as the
/// engine. Integrators can use `installer_wizard_with_selections` to inject the
/// engine's authoritative defaults, including command-line selections.
pub fn installer_wizard(
    manifest: &Manifest,
    default_dir: &Path,
    perform: InstallCallback,
) -> Result<()> {
    let (components, tasks) = flow::default_selections(manifest, None)?;
    installer_wizard_with_selections(
        manifest,
        WizardSelection {
            target_dir: default_dir.to_path_buf(),
            components,
            tasks,
        },
        perform,
    )
}

pub fn installer_wizard_with_selections(
    manifest: &Manifest,
    selections: WizardSelection,
    perform: InstallCallback,
) -> Result<()> {
    #[cfg(windows)]
    {
        windows::installer_wizard(manifest, selections, perform)
    }
    #[cfg(not(windows))]
    {
        let _ = (manifest, selections, perform);
        anyhow::bail!(
            "The native installer wizard requires Windows. Use the ironsetup CLI on this platform."
        )
    }
}

pub fn compiler_studio(initial_script: Option<PathBuf>, compile: CompileCallback) -> Result<()> {
    #[cfg(windows)]
    {
        windows::compiler_studio(initial_script, compile)
    }
    #[cfg(not(windows))]
    {
        let _ = (initial_script, compile);
        anyhow::bail!(
            "IronSetup Studio requires Windows. Use 'ironsetup compile' on this platform."
        )
    }
}

pub fn confirm_uninstall(app_name: &str, target: &Path) -> Result<bool> {
    #[cfg(windows)]
    {
        windows::confirm_uninstall(app_name, target)
    }
    #[cfg(not(windows))]
    {
        let _ = (app_name, target);
        anyhow::bail!("The native uninstall confirmation requires Windows. Use the ironsetup CLI.")
    }
}

pub fn show_error(message: &str) {
    #[cfg(windows)]
    windows::show_error(message);
    #[cfg(not(windows))]
    eprintln!("IronSetup: {message}");
}

pub fn show_information(message: &str) {
    #[cfg(windows)]
    windows::show_information(message);
    #[cfg(not(windows))]
    println!("{message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_wizard_outcomes_remain_distinct_through_context() {
        let cancelled = anyhow::Error::new(WizardError::Cancelled).context("Wizard closed");
        assert!(is_cancelled(&cancelled));
        assert!(!is_operation_reported(&cancelled));
        let reported = anyhow::Error::new(WizardError::OperationFailed("File locked".into()))
            .context("Install failed");
        assert!(is_operation_reported(&reported));
        assert!(!is_cancelled(&reported));
        let startup = anyhow::anyhow!("Cannot create native window");
        assert!(!is_cancelled(&startup));
        assert!(!is_operation_reported(&startup));
    }
}
