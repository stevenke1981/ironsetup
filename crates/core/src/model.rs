use serde::{Deserialize, Serialize};

pub const FORMAT_VERSION: u32 = 1;
pub const MAX_PACKAGE_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_FILES: usize = 100_000;
pub const STATE_DIR: &str = ".ironsetup";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub publisher: String,
    pub default_dir: String,
    pub default_group: String,
    /// "lowest" is per-user; "admin" requires an elevated process.
    pub privileges: String,
    pub uninstallable: bool,
    /// "zip" (deflate) or "none".
    pub compression: String,
    /// "x64compatible" or empty. This release builds an x64 Windows stub.
    #[serde(default)]
    pub architectures_allowed: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conditions {
    #[serde(default)]
    pub components: String,
    #[serde(default)]
    pub tasks: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestFile {
    /// Portable relative path under the installation directory, with '/'.
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub executable: bool,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub conditions: Conditions,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DirectoryEntry {
    pub path: String,
    #[serde(default)]
    pub conditions: Conditions,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub conditions: Conditions,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupType {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub custom: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IconEntry {
    pub name: String,
    pub filename: String,
    #[serde(default)]
    pub parameters: String,
    #[serde(default)]
    pub working_dir: String,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub icon_filename: String,
    #[serde(default)]
    pub conditions: Conditions,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryEntry {
    pub root: String,
    pub subkey: String,
    pub value_type: String,
    #[serde(default)]
    pub value_name: String,
    #[serde(default)]
    pub value_data: String,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub conditions: Conditions,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunEntry {
    pub filename: String,
    #[serde(default)]
    pub parameters: String,
    #[serde(default)]
    pub working_dir: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub conditions: Conditions,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format_version: u32,
    pub app: AppInfo,
    #[serde(default)]
    pub files: Vec<ManifestFile>,
    #[serde(default)]
    pub dirs: Vec<DirectoryEntry>,
    #[serde(default)]
    pub components: Vec<Choice>,
    #[serde(default)]
    pub tasks: Vec<Choice>,
    #[serde(default)]
    pub types: Vec<SetupType>,
    #[serde(default)]
    pub icons: Vec<IconEntry>,
    #[serde(default)]
    pub registry: Vec<RegistryEntry>,
    #[serde(default)]
    pub run: Vec<RunEntry>,
    #[serde(default)]
    pub license_text: Option<String>,
    #[serde(default)]
    pub info_before: Option<String>,
    #[serde(default)]
    pub info_after: Option<String>,
}

/// Payload vectors align exactly with manifest.files, verified before installing.
#[derive(Clone, Debug)]
pub struct Package {
    pub manifest: Manifest,
    pub payloads: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub file: String,
    pub line: usize,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct Progress {
    pub stage: String,
    pub item: String,
    pub completed: u64,
    pub total: u64,
}
