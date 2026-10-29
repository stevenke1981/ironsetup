use crate::{
    parse::{self, Entry, Parsed},
    preprocess::{host_path, preprocess_lines},
    CompileOptions, CompiledScript,
};
use anyhow::{bail, Context, Result};
use ironsetup_core::*;
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

mod files;

const SETUP_KEYS: &[&str] = &[
    "appid",
    "appname",
    "appversion",
    "apppublisher",
    "defaultdirname",
    "defaultgroupname",
    "privilegesrequired",
    "uninstallable",
    "compression",
    "solidcompression",
    "outputdir",
    "outputbasefilename",
    "sourcedir",
    "licensefile",
    "infobeforefile",
    "infoafterfile",
    "architecturesallowed",
];
const CONSTANTS: &[&str] = &[
    "app",
    "group",
    "userdesktop",
    "autodesktop",
    "userprograms",
    "localappdata",
    "userappdata",
    "userpf",
    "autopf",
    "pf",
    "pf64",
    "uninstallexe",
];

pub fn compile_script(path: &Path, options: &CompileOptions) -> Result<CompiledScript> {
    let script =
        fs::canonicalize(path).with_context(|| format!("Cannot open script {}", path.display()))?;
    let parsed = parse::parse(preprocess_lines(&script, options)?)?;
    for (key, (_, line)) in &parsed.setup {
        if !SETUP_KEYS.contains(&key.as_str()) {
            return Err(line.error(format!("Unsupported [Setup] directive {key}; this release does not silently ignore directives")));
        }
    }
    let script_dir = script.parent().context("Script has no parent directory")?;
    let source_dir = resolve_host_path(script_dir, parsed.setting("sourcedir").unwrap_or("."))
        .map_err(|e| parsed.setting_error("sourcedir", e))?;
    let app_name = parsed.required("appname")?.to_owned();
    let app_version = parsed.required("appversion")?.to_owned();
    let app_id = parsed.setting("appid").unwrap_or(&app_name).to_owned();
    if app_id.trim().is_empty() {
        return Err(parsed.setting_error("appid", "AppId cannot be empty"));
    }
    for (name, value) in [
        ("appname", &app_name),
        ("appversion", &app_version),
        ("appid", &app_id),
    ] {
        if value.len() > 4096 {
            return Err(parsed.setting_error(name, "Value exceeds 4096 bytes"));
        }
    }
    let default_dir = normalize_constants(parsed.required("defaultdirname")?)
        .map_err(|e| parsed.setting_error("defaultdirname", e))?;
    validate_default_dir(&default_dir).map_err(|e| parsed.setting_error("defaultdirname", e))?;
    let privileges = parsed
        .setting("privilegesrequired")
        .unwrap_or("lowest")
        .to_ascii_lowercase();
    if !["lowest", "admin"].contains(&privileges.as_str()) {
        return Err(parsed.setting_error("privilegesrequired", "Supported values: lowest, admin"));
    }
    let compression = parsed
        .setting("compression")
        .unwrap_or("zip")
        .to_ascii_lowercase();
    if !["zip", "none"].contains(&compression.as_str()) {
        return Err(parsed.setting_error("compression", "Supported values: zip (deflate), none; LZMA/LZMA2 and solid compression are not implemented"));
    }
    if let Some(solid) = parsed.setting("solidcompression") {
        if parse_bool(solid).map_err(|e| parsed.setting_error("solidcompression", e))? {
            return Err(parsed.setting_error(
                "solidcompression",
                "SolidCompression=yes is not implemented; use no",
            ));
        }
    }
    let architecture = parsed
        .setting("architecturesallowed")
        .unwrap_or("")
        .to_ascii_lowercase();
    if !["", "x64compatible"].contains(&architecture.as_str()) {
        return Err(parsed.setting_error(
            "architecturesallowed",
            "Only x64compatible is supported by the x64 Windows runtime",
        ));
    }
    let uninstallable = match parsed.setting("uninstallable") {
        Some(value) => parse_bool(value).map_err(|e| parsed.setting_error("uninstallable", e))?,
        None => true,
    };
    let default_group = parsed
        .setting("defaultgroupname")
        .unwrap_or(&app_name)
        .to_owned();
    let types = compile_types(&parsed)?;
    let type_names: BTreeSet<String> = types.iter().map(|t| t.name.clone()).collect();
    let components = compile_components(&parsed, &type_names)?;
    let component_names: BTreeSet<String> = components.iter().map(|c| c.name.clone()).collect();
    let tasks = compile_tasks(&parsed, &component_names)?;
    let task_names: BTreeSet<String> = tasks.iter().map(|t| t.name.clone()).collect();
    let (manifest_files, payloads, implicit_dirs) =
        files::compile_files(&parsed, &source_dir, &component_names, &task_names)?;
    let dirs = compile_dirs(&parsed, &component_names, &task_names, implicit_dirs)?;
    let icons = compile_icons(
        &parsed,
        &component_names,
        &task_names,
        uninstallable,
        &default_group,
    )?;
    let registry = compile_registry(&parsed, &component_names, &task_names, &privileges)?;
    let run = compile_run(&parsed, &component_names, &task_names)?;
    let app = AppInfo {
        id: app_id,
        name: app_name,
        version: app_version,
        publisher: parsed.setting("apppublisher").unwrap_or("").to_owned(),
        default_dir,
        default_group,
        privileges,
        uninstallable,
        compression,
        architectures_allowed: architecture,
    };
    let manifest = Manifest {
        format_version: FORMAT_VERSION,
        app,
        files: manifest_files,
        dirs,
        components,
        tasks,
        types,
        icons,
        registry,
        run,
        license_text: read_optional_text(&parsed, &source_dir, "licensefile")?,
        info_before: read_optional_text(&parsed, &source_dir, "infobeforefile")?,
        info_after: read_optional_text(&parsed, &source_dir, "infoafterfile")?,
    };
    if parsed
        .setting("outputdir")
        .is_some_and(|p| p.to_ascii_lowercase().starts_with("userdocs:"))
    {
        return Err(parsed.setting_error(
            "outputdir",
            "The userdocs: special prefix is not supported; use an explicit output path",
        ));
    }
    let output_dir =
        resolve_host_path(&source_dir, parsed.setting("outputdir").unwrap_or("Output"))
            .map_err(|e| parsed.setting_error("outputdir", e))?;
    let output_basename = parsed
        .setting("outputbasefilename")
        .unwrap_or("setup")
        .to_owned();
    validate_relative_path(&output_basename)
        .map_err(|e| parsed.setting_error("outputbasefilename", e))?;
    if output_basename.contains(['/', '\\']) {
        return Err(parsed.setting_error(
            "outputbasefilename",
            "OutputBaseFilename must be a filename; use OutputDir for its directory",
        ));
    }
    let mut warnings = Vec::new();
    for (key, message) in [
        ("privilegesrequired", "IronSetup default: PrivilegesRequired=lowest (Inno Setup defaults to admin). Set it explicitly to document intent."),
        ("compression", "IronSetup default: Compression=zip (deflate). Set it explicitly; LZMA/LZMA2 are not supported."),
    ] {
        if parsed.setting(key).is_none() {
            warnings.push(Diagnostic { file: script.display().to_string(), line: 0, message: message.to_owned() });
        }
    }
    if manifest
        .run
        .iter()
        .any(|r| r.flags.iter().any(|f| f == "nowait"))
    {
        warnings.push(Diagnostic { file: script.display().to_string(), line: 0,
            message: "[Run] nowait reports launch success only; external program side effects cannot be rolled back.".to_owned() });
    }
    let package = Package { manifest, payloads };
    ironsetup_core::bundle::validate_package(&package)
        .with_context(|| format!("{}: package validation failed", script.display()))?;
    Ok(CompiledScript {
        package,
        output_dir,
        output_basename,
        warnings,
    })
}

fn resolve_host_path(base: &Path, value: &str) -> Result<PathBuf> {
    let path = host_path(value)?;
    Ok(if path.is_absolute() {
        path
    } else {
        base.join(path)
    })
}

fn parse_bool(value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "yes" | "true" | "1" => Ok(true),
        "no" | "false" | "0" => Ok(false),
        _ => bail!("Expected yes or no, got {value}"),
    }
}

fn compile_types(parsed: &Parsed) -> Result<Vec<SetupType>> {
    let mut seen = BTreeSet::new();
    let mut types = Vec::new();
    let mut custom_count = 0;
    for entry in parsed.entries("types") {
        entry.allow(&["name", "description", "flags"])?;
        let name = choice_name(entry)?;
        if !seen.insert(name.clone()) {
            return Err(entry.source.error(format!("Duplicate type {name}")));
        }
        let flags = entry.flags(&["iscustom"])?;
        let custom = flags.iter().any(|f| f == "iscustom");
        if custom {
            custom_count += 1;
        }
        if custom_count > 1 {
            return Err(entry.source.error("Only one custom setup type is allowed"));
        }
        types.push(SetupType {
            name,
            description: entry.required("description")?.to_owned(),
            custom,
        });
    }
    Ok(types)
}

fn compile_components(parsed: &Parsed, types: &BTreeSet<String>) -> Result<Vec<Choice>> {
    let mut seen = BTreeSet::new();
    let mut choices = Vec::new();
    for entry in parsed.entries("components") {
        entry.allow(&["name", "description", "types", "flags"])?;
        let name = choice_name(entry)?;
        if !seen.insert(name.clone()) {
            return Err(entry.source.error(format!("Duplicate component {name}")));
        }
        let type_names: Vec<String> = entry
            .get("types")
            .split_whitespace()
            .map(str::to_ascii_lowercase)
            .collect();
        for name in &type_names {
            if !types.contains(name) {
                return Err(entry.source.error(format!("Unknown setup type: {name}")));
            }
        }
        choices.push(Choice {
            name,
            description: entry.required("description")?.to_owned(),
            types: type_names,
            flags: entry.flags(&["fixed"])?,
            conditions: Conditions::default(),
        });
    }
    Ok(choices)
}

fn compile_tasks(parsed: &Parsed, components: &BTreeSet<String>) -> Result<Vec<Choice>> {
    let mut seen = BTreeSet::new();
    let mut tasks = Vec::new();
    for entry in parsed.entries("tasks") {
        entry.allow(&["name", "description", "components", "flags"])?;
        let name = choice_name(entry)?;
        if !seen.insert(name.clone()) {
            return Err(entry.source.error(format!("Duplicate task {name}")));
        }
        tasks.push(Choice {
            name,
            description: entry.required("description")?.to_owned(),
            types: Vec::new(),
            flags: entry.flags(&["unchecked"])?,
            conditions: Conditions {
                components: condition(entry, "components", components)?,
                tasks: String::new(),
            },
        });
    }
    Ok(tasks)
}

fn choice_name(entry: &Entry) -> Result<String> {
    let name = entry.required("name")?.to_ascii_lowercase();
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
        || ["and", "or", "not"].contains(&name.as_str())
    {
        return Err(entry.source.error("Choice names must contain only letters, digits, _, -, or .; hierarchical choices and boolean keywords are not supported"));
    }
    Ok(name)
}

fn compile_dirs(
    parsed: &Parsed,
    components: &BTreeSet<String>,
    tasks: &BTreeSet<String>,
    mut dirs: Vec<DirectoryEntry>,
) -> Result<Vec<DirectoryEntry>> {
    for entry in parsed.entries("dirs") {
        entry.allow(&["name", "components", "tasks"])?;
        let path =
            app_relative(entry.required("name")?, true).map_err(|e| entry.source.error(e))?;
        if path.is_empty() {
            return Err(entry
                .source
                .error("[Dirs] must name a subdirectory under {app}"));
        }
        dirs.push(DirectoryEntry {
            path,
            conditions: conditions(entry, components, tasks)?,
        });
    }
    dirs.sort_by(|a, b| {
        a.path
            .to_lowercase()
            .cmp(&b.path.to_lowercase())
            .then_with(|| a.conditions.components.cmp(&b.conditions.components))
            .then_with(|| a.conditions.tasks.cmp(&b.conditions.tasks))
    });
    dirs.dedup_by(|a, b| {
        a.path.eq_ignore_ascii_case(&b.path)
            && a.conditions.components == b.conditions.components
            && a.conditions.tasks == b.conditions.tasks
    });
    Ok(dirs)
}

fn compile_icons(
    parsed: &Parsed,
    components: &BTreeSet<String>,
    tasks: &BTreeSet<String>,
    uninstallable: bool,
    default_group: &str,
) -> Result<Vec<IconEntry>> {
    let mut seen = BTreeSet::new();
    let mut icons = Vec::new();
    for entry in parsed.entries("icons") {
        entry.allow(&[
            "name",
            "filename",
            "parameters",
            "workingdir",
            "comment",
            "iconfilename",
            "components",
            "tasks",
        ])?;
        let name =
            normalize_constants(entry.required("name")?).map_err(|e| entry.source.error(e))?;
        let prefix = ["{group}", "{userdesktop}", "{autodesktop}", "{userprograms}"].iter().find(|prefix| name.starts_with(**prefix))
            .ok_or_else(|| entry.source.error("Icon Name must start with {group}, {userdesktop}, {autodesktop}, or {userprograms}"))?;
        let suffix = &name[prefix.len()..];
        let relative = suffix.strip_prefix(['/', '\\']).unwrap_or(suffix);
        if relative.is_empty() || !matches!(name.as_bytes().get(prefix.len()), Some(b'/' | b'\\')) {
            return Err(entry
                .source
                .error("Icon Name requires a relative filename after its directory constant"));
        }
        if relative.contains('{') {
            return Err(entry
                .source
                .error("Nested runtime constants in an icon name are not supported"));
        }
        validate_relative_path(relative).map_err(|e| entry.source.error(e))?;
        if *prefix == "{group}" {
            if default_group.contains('{') {
                return Err(entry.source.error(
                    "DefaultGroupName must be literal (preprocessor definitions are allowed)",
                ));
            }
            validate_relative_path(default_group)
                .map_err(|e| entry.source.error(format!("Invalid DefaultGroupName: {e}")))?;
        }
        if !seen.insert(name.to_lowercase()) {
            return Err(entry
                .source
                .error(format!("Duplicate icon destination: {name}")));
        }
        let filename =
            normalize_constants(entry.required("filename")?).map_err(|e| entry.source.error(e))?;
        if filename == "{uninstallexe}" {
            if !uninstallable {
                return Err(entry
                    .source
                    .error("{uninstallexe} requires Uninstallable=yes"));
            }
        } else {
            app_relative(&filename, false).map_err(|e| entry.source.error(e))?;
        }
        let working_dir =
            normalize_constants(entry.get("workingdir")).map_err(|e| entry.source.error(e))?;
        if !working_dir.is_empty() {
            app_relative(&working_dir, true).map_err(|e| entry.source.error(e))?;
        }
        let icon_filename =
            normalize_constants(entry.get("iconfilename")).map_err(|e| entry.source.error(e))?;
        if !icon_filename.is_empty() {
            app_relative(&icon_filename, false).map_err(|e| entry.source.error(e))?;
        }
        icons.push(IconEntry {
            name,
            filename,
            working_dir,
            icon_filename,
            parameters: normalize_constants(entry.get("parameters"))
                .map_err(|e| entry.source.error(e))?,
            comment: entry.get("comment").to_owned(),
            conditions: conditions(entry, components, tasks)?,
        });
    }
    Ok(icons)
}

fn compile_registry(
    parsed: &Parsed,
    components: &BTreeSet<String>,
    tasks: &BTreeSet<String>,
    privileges: &str,
) -> Result<Vec<RegistryEntry>> {
    let mut seen = BTreeSet::new();
    let mut entries = Vec::new();
    for entry in parsed.entries("registry") {
        entry.allow(&[
            "root",
            "subkey",
            "valuetype",
            "valuename",
            "valuedata",
            "flags",
            "components",
            "tasks",
        ])?;
        let root = entry.required("root")?.to_ascii_uppercase();
        if !["HKCU", "HKLM", "HKA"].contains(&root.as_str()) {
            return Err(entry
                .source
                .error("Registry Root must be HKCU, HKLM, or HKA"));
        }
        if root == "HKLM" && privileges != "admin" {
            return Err(entry
                .source
                .error("HKLM registry entries require PrivilegesRequired=admin"));
        }
        let subkey = entry.required("subkey")?.to_owned();
        if subkey.starts_with(['\\', '/'])
            || subkey.contains(['{', '}', '/'])
            || subkey
                .split('\\')
                .any(|s| s.is_empty() || s == "." || s == "..")
        {
            return Err(entry.source.error("Registry Subkey must be a literal nonempty key path, without constants or empty/path-traversal segments"));
        }
        let value_type = entry.required("valuetype")?.to_ascii_lowercase();
        let raw_data = entry.get("valuedata");
        let value_data = match value_type.as_str() {
            "string" | "expandsz" => {
                normalize_constants(raw_data).map_err(|e| entry.source.error(e))?
            }
            "dword" => parse_unsigned(raw_data, u32::MAX as u64)
                .map_err(|e| entry.source.error(e))?
                .to_string(),
            "qword" => parse_unsigned(raw_data, u64::MAX)
                .map_err(|e| entry.source.error(e))?
                .to_string(),
            "binary" => {
                let hex: String = raw_data.chars().filter(|c| !c.is_whitespace()).collect();
                if hex.len() & 1 != 0 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err(entry
                        .source
                        .error("Binary ValueData must contain complete hexadecimal byte pairs"));
                }
                hex.to_ascii_uppercase()
            }
            _ => {
                return Err(entry
                    .source
                    .error("Supported registry ValueType: string, expandsz, dword, qword, binary"))
            }
        };
        let value_name = entry.get("valuename").to_owned();
        if value_name.contains('{') {
            return Err(entry
                .source
                .error("Registry ValueName constants are not supported"));
        }
        let identity_root = if root == "HKA" {
            if privileges == "admin" {
                "HKLM"
            } else {
                "HKCU"
            }
        } else {
            &root
        };
        let identity = format!(
            "{}\\{}\0{}",
            identity_root,
            subkey.to_lowercase(),
            value_name.to_lowercase()
        );
        if !seen.insert(identity) {
            return Err(entry.source.error("Duplicate registry value destination"));
        }
        entries.push(RegistryEntry {
            root,
            subkey,
            value_type,
            value_name,
            value_data,
            flags: entry.flags(&["uninsdeletevalue"])?,
            conditions: conditions(entry, components, tasks)?,
        });
    }
    Ok(entries)
}

fn parse_unsigned(value: &str, max: u64) -> Result<u64> {
    let value = value.trim();
    let result = if let Some(hex) = value
        .strip_prefix('$')
        .or_else(|| value.strip_prefix("0x"))
        .or_else(|| value.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).context("Invalid hexadecimal registry integer")?
    } else {
        value
            .parse::<u64>()
            .context("Invalid unsigned registry integer")?
    };
    if result > max {
        bail!("Registry integer is out of range");
    }
    Ok(result)
}

fn compile_run(
    parsed: &Parsed,
    components: &BTreeSet<String>,
    tasks: &BTreeSet<String>,
) -> Result<Vec<RunEntry>> {
    let mut runs = Vec::new();
    for entry in parsed.entries("run") {
        entry.allow(&[
            "filename",
            "parameters",
            "workingdir",
            "description",
            "flags",
            "components",
            "tasks",
        ])?;
        let filename =
            normalize_constants(entry.required("filename")?).map_err(|e| entry.source.error(e))?;
        app_relative(&filename, false).map_err(|e| entry.source.error(e))?;
        let flags = entry.flags(&["skipifsilent", "nowait", "runhidden", "waituntilterminated"])?;
        if flags.iter().any(|f| f == "nowait") && flags.iter().any(|f| f == "waituntilterminated") {
            return Err(entry
                .source
                .error("nowait and waituntilterminated cannot be combined"));
        }
        let working_dir =
            normalize_constants(entry.get("workingdir")).map_err(|e| entry.source.error(e))?;
        if !working_dir.is_empty() {
            app_relative(&working_dir, true).map_err(|e| entry.source.error(e))?;
        }
        runs.push(RunEntry {
            filename,
            working_dir,
            flags,
            parameters: normalize_constants(entry.get("parameters"))
                .map_err(|e| entry.source.error(e))?,
            description: entry.get("description").to_owned(),
            conditions: conditions(entry, components, tasks)?,
        });
    }
    Ok(runs)
}

fn read_optional_text(parsed: &Parsed, base: &Path, directive: &str) -> Result<Option<String>> {
    let Some(value) = parsed.setting(directive) else {
        return Ok(None);
    };
    if value.is_empty() {
        return Err(parsed.setting_error(directive, "Text filename cannot be empty"));
    }
    let path = resolve_host_path(base, value).map_err(|e| parsed.setting_error(directive, e))?;
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .and_then(|f| f.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes))
        .map_err(|e| {
            parsed.setting_error(directive, format!("Cannot read {}: {e}", path.display()))
        })?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(parsed.setting_error(directive, "Text file exceeds 4 MiB"));
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| parsed.setting_error(directive, "Text file must be UTF-8"))?;
    let text = text.trim_start_matches('\u{feff}').to_owned();
    if text.starts_with("{\\rtf") {
        return Err(parsed.setting_error(
            directive,
            "RTF rendering is not implemented; use a plain UTF-8 text file",
        ));
    }
    if text.contains('\0') {
        return Err(parsed.setting_error(directive, "Text file contains NUL characters"));
    }
    Ok(Some(text))
}

pub(super) fn conditions(
    entry: &Entry,
    components: &BTreeSet<String>,
    tasks: &BTreeSet<String>,
) -> Result<Conditions> {
    Ok(Conditions {
        components: condition(entry, "components", components)?,
        tasks: condition(entry, "tasks", tasks)?,
    })
}

fn condition(entry: &Entry, key: &str, choices: &BTreeSet<String>) -> Result<String> {
    let raw = entry.get(key);
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in raw.chars().chain(std::iter::once(' ')) {
        if ch.is_whitespace() || ch == '(' || ch == ')' {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current).to_ascii_lowercase());
            }
            if ch == '(' || ch == ')' {
                tokens.push(ch.to_string());
            }
        } else if ch.is_ascii_alphanumeric() || "_.-".contains(ch) {
            current.push(ch);
        } else {
            return Err(entry
                .source
                .error(format!("Unsupported character in {key} expression: {ch}")));
        }
    }
    if tokens.is_empty() {
        return Ok(String::new());
    }
    let normalized = tokens.join(" ");
    ironsetup_core::evaluate_condition(&normalized, &BTreeSet::new())
        .map_err(|e| entry.source.error(format!("Invalid {key} expression: {e}")))?;
    for token in &tokens {
        if !["and", "or", "not", "(", ")"].contains(&token.as_str()) && !choices.contains(token) {
            return Err(entry.source.error(format!("Unknown {key} choice: {token}")));
        }
    }
    Ok(normalized)
}

pub(super) fn app_relative(value: &str, allow_empty: bool) -> Result<String> {
    if !value
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("{app}"))
    {
        bail!("Destination must start with {{app}}; writes outside the installation directory are not supported");
    }
    let suffix = &value[5..];
    if !suffix.is_empty() && !suffix.starts_with(['/', '\\']) {
        bail!("Expected a directory separator after {{app}}");
    }
    // Strip exactly one separator, so duplicate separators cannot hide an absolute path.
    let relative = if suffix.is_empty() { "" } else { &suffix[1..] };
    if relative.is_empty() && allow_empty {
        return Ok(String::new());
    }
    if relative.contains('{') {
        bail!("Nested runtime constants in an {{app}} relative path are not supported");
    }
    let path = relative.replace('\\', "/");
    validate_relative_path(&path)?;
    Ok(path)
}

fn normalize_constants(value: &str) -> Result<String> {
    let mut out = String::new();
    let mut rest = value;
    while let Some(index) = rest.find('{') {
        out.push_str(&rest[..index]);
        rest = &rest[index..];
        if let Some(after) = rest.strip_prefix("{{") {
            out.push_str("{{");
            rest = after;
            continue;
        }
        let end = rest.find('}').context("Unterminated runtime constant")?;
        let name = rest[1..end].to_ascii_lowercase();
        if !CONSTANTS.contains(&name.as_str()) {
            bail!("Unsupported runtime constant {{{name}}}");
        }
        out.push('{');
        out.push_str(&name);
        out.push('}');
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn validate_default_dir(value: &str) -> Result<()> {
    if let Some(end) = value.find('}') {
        let prefix = &value[..=end];
        if ![
            "{autopf}",
            "{userpf}",
            "{pf}",
            "{pf64}",
            "{localappdata}",
            "{userappdata}",
        ]
        .contains(&prefix)
        {
            bail!("DefaultDirName must use {{autopf}}, {{userpf}}, {{pf}}, {{pf64}}, {{localappdata}}, {{userappdata}}, or a literal absolute path");
        }
        let suffix = &value[end + 1..];
        if !suffix.starts_with(['/', '\\']) {
            bail!("DefaultDirName requires an application subdirectory after its base constant");
        }
        let relative = &suffix[1..];
        if relative.contains('{') {
            bail!("Nested runtime constants in DefaultDirName are not supported");
        }
        validate_relative_path(relative)?;
    } else if let Some(relative) = value.strip_prefix('/') {
        validate_relative_path(relative)?;
    } else if value.as_bytes().get(1) == Some(&b':')
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
        && matches!(value.as_bytes().get(2), Some(b'/' | b'\\'))
    {
        validate_relative_path(&value[3..])?;
    } else {
        bail!("DefaultDirName must be an absolute path or a supported directory constant plus an application subdirectory");
    }
    Ok(())
}
