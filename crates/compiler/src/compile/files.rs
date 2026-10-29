use super::{app_relative, conditions, resolve_host_path};
use crate::parse::{Entry, Parsed};
use anyhow::{bail, Context, Result};
use glob::{MatchOptions, Pattern};
use ironsetup_core::{
    sha256, validate_relative_path, DirectoryEntry, ManifestFile, MAX_FILES, MAX_PACKAGE_BYTES,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

const MATCH: MatchOptions = MatchOptions {
    case_sensitive: false,
    require_literal_separator: true,
    require_literal_leading_dot: false,
};
const MAX_VISITED: usize = 200_000;

struct Candidate {
    source: PathBuf,
    relative: String,
}
struct Found {
    files: Vec<Candidate>,
    dirs: Vec<String>,
    visited: usize,
}

type FileCompilation = (Vec<ManifestFile>, Vec<Vec<u8>>, Vec<DirectoryEntry>);

pub(super) fn compile_files(
    parsed: &Parsed,
    source_dir: &Path,
    components: &BTreeSet<String>,
    tasks: &BTreeSet<String>,
) -> Result<FileCompilation> {
    let mut result: BTreeMap<String, (ManifestFile, Vec<u8>)> = BTreeMap::new();
    let mut dirs = Vec::new();
    let mut bytes_total: u64 = 0;
    for entry in parsed.entries("files") {
        entry.allow(&[
            "source",
            "destdir",
            "destname",
            "excludes",
            "flags",
            "components",
            "tasks",
        ])?;
        let flags = entry.flags(&[
            "ignoreversion",
            "onlyifdoesntexist",
            "uninsneveruninstall",
            "recursesubdirs",
            "createallsubdirs",
            "skipifsourcedoesntexist",
        ])?;
        let has_flag = |flag: &str| flags.iter().any(|f| f == flag);
        if !has_flag("ignoreversion") && !has_flag("onlyifdoesntexist") {
            return Err(entry.source.error("PE file-version comparison is not implemented. Explicitly add Flags: ignoreversion to permit replacement, or onlyifdoesntexist to preserve existing files."));
        }
        if has_flag("createallsubdirs") && !has_flag("recursesubdirs") {
            return Err(entry
                .source
                .error("createallsubdirs requires recursesubdirs"));
        }
        let dest =
            app_relative(entry.required("destdir")?, true).map_err(|e| entry.source.error(e))?;
        let cond = conditions(entry, components, tasks)?;
        let raw_source = entry.required("source")?;
        if raw_source.contains('{') {
            return Err(entry.source.error("Runtime constants in Source are unsupported; external/download files are not implemented"));
        }
        let source =
            resolve_host_path(source_dir, raw_source).map_err(|e| entry.source.error(e))?;
        let parent = source.parent().context("Source path has no parent")?;
        if has_directory_wildcards(parent) {
            return Err(entry.source.error(
                "Wildcards are supported only in the final Source filename, not directory segments",
            ));
        }
        let pattern_text = source
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| entry.source.error("Source filename must be UTF-8"))?;
        let pattern = wildcard_pattern(if pattern_text == "*.*" {
            "*"
        } else {
            pattern_text
        })
        .map_err(|e| entry.source.error(format!("Invalid Source pattern: {e}")))?;
        let excludes = parse_excludes(entry)?;
        let mut found = Found {
            files: Vec::new(),
            dirs: Vec::new(),
            visited: 0,
        };
        if !parent.exists() {
            if has_flag("skipifsourcedoesntexist") {
                continue;
            }
            return Err(entry.source.error(format!(
                "Source directory does not exist: {}",
                parent.display()
            )));
        }
        ironsetup_core::reject_symlink_ancestors(parent).map_err(|e| entry.source.error(e))?;
        visit(
            parent,
            parent,
            &pattern,
            &excludes,
            has_flag("recursesubdirs"),
            has_flag("createallsubdirs"),
            0,
            &mut found,
        )
        .map_err(|e| entry.source.error(e))?;
        found.files.sort_by(|a, b| {
            a.relative
                .to_lowercase()
                .cmp(&b.relative.to_lowercase())
                .then_with(|| a.relative.cmp(&b.relative))
        });
        if found.files.is_empty() && !has_flag("skipifsourcedoesntexist") {
            return Err(entry
                .source
                .error(format!("Source matched no files: {raw_source}")));
        }
        let dest_name = entry.get("destname");
        if !dest_name.is_empty() {
            validate_relative_path(dest_name).map_err(|e| entry.source.error(e))?;
            if dest_name.contains(['/', '\\']) {
                return Err(entry.source.error("DestName must be a single filename"));
            }
            if found.files.len() > 1 {
                return Err(entry.source.error(
                    "DestName cannot rename multiple matching files to the same destination",
                ));
            }
        }
        for candidate in found.files {
            let relative = if dest_name.is_empty() {
                candidate.relative
            } else {
                dest_name.to_owned()
            };
            let path = if dest.is_empty() {
                relative
            } else {
                format!("{dest}/{relative}")
            };
            validate_relative_path(&path).map_err(|e| entry.source.error(e))?;
            let key = path.to_lowercase();
            if result.contains_key(&key) {
                return Err(entry.source.error(format!("Duplicate destination (case-insensitive): {path}; conditional duplicate payloads are not supported")));
            }
            if result.len() >= MAX_FILES {
                return Err(entry
                    .source
                    .error(format!("Package exceeds the {MAX_FILES} file limit")));
            }
            let remaining = MAX_PACKAGE_BYTES
                .checked_sub(bytes_total)
                .context("Package size overflow")?;
            let mut data = Vec::new();
            fs::File::open(&candidate.source)?
                .take(remaining + 1)
                .read_to_end(&mut data)?;
            if data.len() as u64 > remaining {
                return Err(entry
                    .source
                    .error("Uncompressed payload exceeds the 512 MiB package limit"));
            }
            bytes_total += data.len() as u64;
            let meta = fs::metadata(&candidate.source)?;
            #[cfg(unix)]
            let executable = {
                use std::os::unix::fs::PermissionsExt;
                meta.permissions().mode() & 0o111 != 0
                    || path.to_ascii_lowercase().ends_with(".exe")
            };
            #[cfg(not(unix))]
            let executable = {
                let _ = meta;
                path.to_ascii_lowercase().ends_with(".exe")
            };
            let runtime_flags = flags
                .iter()
                .filter(|f| {
                    ["ignoreversion", "onlyifdoesntexist", "uninsneveruninstall"]
                        .contains(&f.as_str())
                })
                .cloned()
                .collect();
            result.insert(
                key,
                (
                    ManifestFile {
                        path,
                        size: data.len() as u64,
                        sha256: sha256(&data),
                        executable,
                        flags: runtime_flags,
                        conditions: cond.clone(),
                    },
                    data,
                ),
            );
        }
        for relative in found.dirs {
            let path = if dest.is_empty() {
                relative
            } else {
                format!("{dest}/{relative}")
            };
            validate_relative_path(&path).map_err(|e| entry.source.error(e))?;
            dirs.push(DirectoryEntry {
                path,
                conditions: cond.clone(),
            });
        }
    }
    for path in result.keys() {
        let mut prefix = String::new();
        let segments: Vec<&str> = path.split('/').collect();
        for segment in &segments[..segments.len() - 1] {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(segment);
            if result.contains_key(&prefix) {
                bail!("Conflicting file and directory destinations: {prefix} and {path}");
            }
        }
    }
    for dir in &dirs {
        if result.contains_key(&dir.path.to_lowercase()) {
            bail!("Destination is both a file and directory: {}", dir.path);
        }
    }
    let (files, payloads) = result.into_values().unzip();
    Ok((files, payloads, dirs))
}

fn has_directory_wildcards(path: &Path) -> bool {
    // Windows canonical paths have a verbatim prefix such as `\\?\C:\` or
    // `\\?\UNC\server\share\`. Its '?' is syntax, not a directory wildcard.
    // Inspect only real path components so '?' in `payload?` is still rejected.
    path.components().any(|component| match component {
        Component::Normal(name) => name.to_string_lossy().contains(['*', '?']),
        _ => false,
    })
}

#[cfg(all(test, windows))]
mod windows_path_tests {
    use super::has_directory_wildcards;
    use std::path::Path;

    #[test]
    fn windows_prefix_markers_are_not_directory_wildcards() {
        for path in [
            r"\\?\C:\payload",
            r"\\?\UNC\server\share\payload",
            r"\\server\share\payload",
        ] {
            assert!(!has_directory_wildcards(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn windows_verbatim_paths_still_reject_wildcard_directory_names() {
        for path in [
            r"\\?\C:\pay?load",
            r"\\?\C:\pay*load",
            r"\\?\UNC\server\share\pay?load",
        ] {
            assert!(has_directory_wildcards(Path::new(path)), "{path}");
        }
    }
}

// ISS patterns recognize * and ?, not glob's bracket-expression extension.
fn wildcard_pattern(raw: &str) -> Result<Pattern, glob::PatternError> {
    let mut escaped = String::new();
    for ch in raw.chars() {
        if ch == '*' || ch == '?' {
            escaped.push(ch);
        } else {
            escaped.push_str(&Pattern::escape(&ch.to_string()));
        }
    }
    Pattern::new(&escaped)
}

fn parse_excludes(entry: &Entry) -> Result<Vec<(Pattern, bool)>> {
    let mut patterns = Vec::new();
    for raw in entry
        .get("excludes")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let normalized = raw.replace('\\', "/");
        let anchored = normalized.starts_with('/');
        let normalized = normalized.trim_start_matches('/');
        let pattern = wildcard_pattern(normalized).map_err(|e| {
            entry
                .source
                .error(format!("Invalid Excludes pattern {raw}: {e}"))
        })?;
        patterns.push((pattern, anchored));
    }
    Ok(patterns)
}

fn excluded(relative: &str, _name: &str, patterns: &[(Pattern, bool)]) -> bool {
    patterns.iter().any(|(p, anchored)| {
        if p.matches_with(relative, MATCH) {
            return true;
        }
        !anchored
            && relative
                .match_indices('/')
                .any(|(index, _)| p.matches_with(&relative[index + 1..], MATCH))
    })
}

#[allow(clippy::too_many_arguments)]
fn visit(
    root: &Path,
    dir: &Path,
    pattern: &Pattern,
    excludes: &[(Pattern, bool)],
    recurse: bool,
    include_dirs: bool,
    depth: usize,
    found: &mut Found,
) -> Result<()> {
    if depth > 128 {
        bail!(
            "Source directory nesting exceeds 128 levels: {}",
            dir.display()
        );
    }
    let mut entries: Vec<_> = fs::read_dir(dir)
        .with_context(|| format!("Cannot read source directory {}", dir.display()))?
        .collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        found.visited += 1;
        if found.visited > MAX_VISITED {
            bail!("Source traversal exceeds {MAX_VISITED} entries");
        }
        let path = entry.path();
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Non-UTF-8 source filename: {}", path.display()))?;
        let relative = path
            .strip_prefix(root)?
            .to_str()
            .context("Non-UTF-8 source path")?
            .replace('\\', "/");
        if excluded(&relative, &name, excludes) {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            bail!("Symbolic links are not packaged: {}", path.display());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if entry.metadata()?.file_attributes() & 0x400 != 0 {
                bail!("Reparse points are not packaged: {}", path.display());
            }
        }
        if kind.is_dir() && recurse {
            if include_dirs {
                found.dirs.push(relative);
            }
            visit(
                root,
                &path,
                pattern,
                excludes,
                recurse,
                include_dirs,
                depth + 1,
                found,
            )?;
        } else if kind.is_file() && pattern.matches_with(&name, MATCH) {
            found.files.push(Candidate {
                source: path,
                relative,
            });
        } else if !kind.is_dir() && !kind.is_file() {
            bail!("Unsupported source file type: {}", path.display());
        }
    }
    Ok(())
}
