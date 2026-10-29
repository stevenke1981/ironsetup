use crate::CompileOptions;
use anyhow::{bail, Context, Result};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

const MAX_SCRIPT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_EXPANDED_BYTES: usize = 16 * 1024 * 1024;
const MAX_INCLUDE_DEPTH: usize = 32;
const MAX_LINES: usize = 200_000;

#[derive(Clone, Debug)]
pub(crate) struct SourceLine {
    pub file: PathBuf,
    pub line: usize,
    pub text: String,
}

impl SourceLine {
    pub fn error(&self, message: impl std::fmt::Display) -> anyhow::Error {
        anyhow::anyhow!("{}:{}: {}", self.file.display(), self.line, message)
    }
}

struct Preprocessor {
    defines: BTreeMap<String, String>,
    protected: BTreeSet<String>,
    stack: Vec<PathBuf>,
    lines: Vec<SourceLine>,
    bytes_read: u64,
    expanded_bytes: usize,
}

pub fn preprocess(path: &Path, options: &CompileOptions) -> Result<String> {
    Ok(preprocess_lines(path, options)?
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n"))
}

pub(crate) fn preprocess_lines(path: &Path, options: &CompileOptions) -> Result<Vec<SourceLine>> {
    for (name, value) in &options.defines {
        if !identifier(name) {
            bail!("Invalid preprocessor definition name: {name}");
        }
        if value.contains(['\n', '\r', '\0']) {
            bail!("Definition {name} must be a single-line literal");
        }
    }
    let mut state = Preprocessor {
        defines: options.defines.clone(),
        protected: options.defines.keys().cloned().collect(),
        stack: Vec::new(),
        lines: Vec::new(),
        bytes_read: 0,
        expanded_bytes: 0,
    };
    state.read(path)?;
    Ok(state.lines)
}

fn identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub(crate) fn host_path(raw: &str) -> Result<PathBuf> {
    #[cfg(not(windows))]
    if raw.as_bytes().get(1) == Some(&b':') || raw.starts_with("\\\\") {
        bail!("Windows absolute source path cannot be read on this host: {raw}; use a relative source path");
    }
    Ok(PathBuf::from(raw.replace('\\', "/")))
}

impl Preprocessor {
    fn read(&mut self, path: &Path) -> Result<()> {
        if self.stack.len() >= MAX_INCLUDE_DEPTH {
            bail!(
                "Include nesting exceeds {MAX_INCLUDE_DEPTH}: {}",
                path.display()
            );
        }
        let canonical = fs::canonicalize(path)
            .with_context(|| format!("Cannot open script {}", path.display()))?;
        if self.stack.contains(&canonical) {
            bail!("Include cycle: {}", path.display());
        }
        let size = fs::metadata(&canonical)?.len();
        self.bytes_read = self
            .bytes_read
            .checked_add(size)
            .context("Script size overflow")?;
        if self.bytes_read > MAX_SCRIPT_BYTES {
            bail!("Combined scripts exceed the 8 MiB limit");
        }
        // Read bounded bytes even if a source grows between metadata and read.
        use std::io::Read;
        let mut bytes = Vec::new();
        fs::File::open(&canonical)?
            .take(MAX_SCRIPT_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_SCRIPT_BYTES {
            bail!("Script exceeds the 8 MiB limit: {}", path.display());
        }
        let text = String::from_utf8(bytes).with_context(|| {
            format!(
                "{}: scripts must be UTF-8 (a UTF-8 BOM is allowed)",
                path.display()
            )
        })?;
        self.stack.push(canonical.clone());
        for (index, raw) in text.trim_start_matches('\u{feff}').lines().enumerate() {
            let source = SourceLine {
                file: canonical.clone(),
                line: index + 1,
                text: raw.to_owned(),
            };
            let trimmed = raw.trim();
            if trimmed.starts_with(';') || trimmed.is_empty() {
                self.push(source)?;
                continue;
            }
            if trimmed.starts_with('#') {
                self.directive(&source, trimmed)?;
            } else {
                let expanded = self.expand(&source, raw)?;
                self.push(SourceLine {
                    text: expanded,
                    ..source
                })?;
            }
        }
        self.stack.pop();
        Ok(())
    }

    fn push(&mut self, line: SourceLine) -> Result<()> {
        self.expanded_bytes = self
            .expanded_bytes
            .checked_add(line.text.len())
            .context("Expanded script size overflow")?;
        if self.expanded_bytes > MAX_EXPANDED_BYTES || self.lines.len() >= MAX_LINES {
            return Err(line.error("Expanded script exceeds the compiler limit"));
        }
        self.lines.push(line);
        Ok(())
    }

    fn directive(&mut self, line: &SourceLine, trimmed: &str) -> Result<()> {
        let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
        let command = &trimmed[..end];
        let rest = trimmed[end..].trim();
        match command {
            "#define" => {
                let split = rest
                    .find(char::is_whitespace)
                    .ok_or_else(|| line.error("Expected #define NAME \"literal value\""))?;
                let name = &rest[..split];
                if !identifier(name) {
                    return Err(line.error(format!("Invalid definition name: {name}")));
                }
                let raw_value = rest[split..].trim();
                let value = if raw_value.starts_with('"') {
                    parse_literal(raw_value).map_err(|e| line.error(e))?
                } else if !raw_value.is_empty()
                    && raw_value
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
                {
                    raw_value.to_owned()
                } else {
                    return Err(line.error("Only quoted string and simple numeric/word literal #define values are supported; ISPP expressions are not evaluated"));
                };
                let value = self.expand(line, &value)?;
                if !self.protected.contains(name) {
                    self.defines.insert(name.to_owned(), value);
                }
            }
            "#include" => {
                let expanded = self.expand(line, rest)?;
                if !expanded.starts_with('"') {
                    return Err(
                        line.error("#include requires a quoted relative or absolute filename")
                    );
                }
                let name = parse_literal(&expanded).map_err(|e| line.error(e))?;
                if name.is_empty() {
                    return Err(line.error("#include filename is empty"));
                }
                let included = host_path(&name).map_err(|e| line.error(e))?;
                let path = if included.is_absolute() {
                    included
                } else {
                    line.file.parent().unwrap_or(Path::new(".")).join(included)
                };
                self.read(&path)
                    .map_err(|e| line.error(format!("while including {name}: {e:#}")))?;
            }
            _ => {
                return Err(line.error(format!(
                    "Unsupported preprocessor directive {command}; supported: #define, #include"
                )))
            }
        }
        Ok(())
    }

    fn expand(&self, line: &SourceLine, text: &str) -> Result<String> {
        let mut out = String::new();
        let mut rest = text;
        while let Some(index) = rest.find("{#") {
            out.push_str(&rest[..index]);
            let after = &rest[index + 2..];
            let end = after
                .find('}')
                .ok_or_else(|| line.error("Unterminated {#NAME} preprocessor reference"))?;
            let name = after[..end].trim();
            let value = self
                .defines
                .get(name)
                .ok_or_else(|| line.error(format!("Undefined preprocessor name: {name}")))?;
            out.push_str(value);
            if out.len() > MAX_EXPANDED_BYTES {
                return Err(line.error("Expanded line exceeds the compiler limit"));
            }
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        Ok(out)
    }
}

pub(crate) fn parse_literal(raw: &str) -> Result<String> {
    let value = raw.trim();
    if !value.starts_with('"') {
        if value.contains('"') {
            bail!("A quoted value must start with a quote; embedded quotes must be doubled inside a quoted value");
        }
        return Ok(value.to_owned());
    }
    let mut chars = value[1..].char_indices().peekable();
    let mut out = String::new();
    while let Some((index, ch)) = chars.next() {
        if ch == '"' {
            if chars.peek().is_some_and(|(_, next)| *next == '"') {
                chars.next();
                out.push('"');
            } else {
                if !value[index + 2..].trim().is_empty() {
                    bail!("Unexpected characters after closing quote");
                }
                return Ok(out);
            }
        } else if ch == '\0' {
            bail!("NUL characters are not supported");
        } else {
            out.push(ch);
        }
    }
    bail!("Unterminated quoted value")
}
