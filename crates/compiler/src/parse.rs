use crate::preprocess::{parse_literal, SourceLine};
use anyhow::Result;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub source: SourceLine,
    pub values: BTreeMap<String, String>,
}

impl Entry {
    pub fn get(&self, name: &str) -> &str {
        self.values.get(name).map(String::as_str).unwrap_or("")
    }
    pub fn required(&self, name: &str) -> Result<&str> {
        let value = self.get(name);
        if value.is_empty() {
            return Err(self
                .source
                .error(format!("Missing required parameter {name}")));
        }
        Ok(value)
    }
    pub fn allow(&self, allowed: &[&str]) -> Result<()> {
        for key in self.values.keys() {
            if !allowed.contains(&key.as_str()) {
                return Err(self.source.error(format!("Unsupported parameter {key}")));
            }
        }
        Ok(())
    }
    pub fn flags(&self, allowed: &[&str]) -> Result<Vec<String>> {
        let mut result = Vec::new();
        for flag in self.get("flags").split_whitespace() {
            let flag = flag.to_ascii_lowercase();
            if !allowed.contains(&flag.as_str()) {
                return Err(self.source.error(format!("Unsupported flag {flag}")));
            }
            if result.contains(&flag) {
                return Err(self.source.error(format!("Duplicate flag {flag}")));
            }
            result.push(flag);
        }
        Ok(result)
    }
}

pub(crate) struct Parsed {
    pub setup: BTreeMap<String, (String, SourceLine)>,
    pub entries: BTreeMap<String, Vec<Entry>>,
}

impl Parsed {
    pub fn entries(&self, name: &str) -> &[Entry] {
        self.entries.get(name).map(Vec::as_slice).unwrap_or(&[])
    }
    pub fn setting(&self, name: &str) -> Option<&str> {
        self.setup.get(name).map(|(s, _)| s.as_str())
    }
    pub fn required(&self, name: &str) -> Result<&str> {
        match self.setup.get(name) {
            Some((s, _)) if !s.is_empty() => Ok(s),
            Some((_, line)) => Err(line.error(format!("[Setup] {name} cannot be empty"))),
            None => anyhow::bail!("Missing required [Setup] directive {name}"),
        }
    }
    pub fn setting_error(&self, name: &str, message: impl std::fmt::Display) -> anyhow::Error {
        self.setup
            .get(name)
            .map(|(_, line)| line.error(&message))
            .unwrap_or_else(|| anyhow::anyhow!("[Setup] {name}: {message}"))
    }
}

pub(crate) fn parse(lines: Vec<SourceLine>) -> Result<Parsed> {
    let mut out = Parsed {
        setup: BTreeMap::new(),
        entries: BTreeMap::new(),
    };
    let mut section = String::new();
    for line in lines {
        let text = line.text.trim();
        if text.is_empty() || text.starts_with(';') {
            continue;
        }
        if text.contains('\0') {
            return Err(line.error("NUL character is not supported"));
        }
        if text.starts_with('[') {
            let end = text
                .find(']')
                .ok_or_else(|| line.error("Unterminated section header"))?;
            if !text[end + 1..].trim().is_empty() {
                return Err(line.error(
                    "Unexpected text after section header; comments must occupy their own line",
                ));
            }
            section = text[1..end].trim().to_ascii_lowercase();
            if ![
                "setup",
                "files",
                "dirs",
                "types",
                "components",
                "tasks",
                "icons",
                "registry",
                "run",
            ]
            .contains(&section.as_str())
            {
                let suffix = if section == "code" {
                    " (Pascal Script is not implemented)"
                } else {
                    ""
                };
                return Err(line.error(format!("Unsupported section [{section}]{suffix}")));
            }
            continue;
        }
        if section.is_empty() {
            return Err(line.error("Entry appears before any section header"));
        }
        if section == "setup" {
            let (key, value) = text
                .split_once('=')
                .ok_or_else(|| line.error("[Setup] entries require Directive=Value"))?;
            let key = key.trim().to_ascii_lowercase();
            if key.is_empty() {
                return Err(line.error("Empty directive name"));
            }
            let value = parse_literal(value).map_err(|e| line.error(e))?;
            if out
                .setup
                .insert(key.clone(), (value, line.clone()))
                .is_some()
            {
                return Err(line.error(format!("Duplicate [Setup] directive {key}")));
            }
        } else {
            let entry = parse_entry(&line)?;
            out.entries.entry(section.clone()).or_default().push(entry);
        }
    }
    Ok(out)
}

fn parse_entry(source: &SourceLine) -> Result<Entry> {
    let text = &source.text;
    let mut segments = Vec::new();
    let mut quoted = false;
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((index, ch)) = chars.next() {
        if ch == '"' {
            if quoted && chars.peek().is_some_and(|(_, next)| *next == '"') {
                chars.next();
            } else {
                quoted = !quoted;
            }
        } else if ch == ';' && !quoted {
            segments.push(&text[start..index]);
            start = index + 1;
        }
    }
    if quoted {
        return Err(source.error("Unterminated quoted value"));
    }
    segments.push(&text[start..]);
    let mut values = BTreeMap::new();
    for (index, segment) in segments.iter().enumerate() {
        let segment = segment.trim();
        if segment.is_empty() && index + 1 == segments.len() {
            continue;
        }
        let (key, raw) = segment.split_once(':').ok_or_else(|| {
            source.error("Section parameters require Name: Value, separated by semicolons")
        })?;
        let key = key.trim().to_ascii_lowercase();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(source.error("Invalid parameter name"));
        }
        let value = parse_literal(raw).map_err(|e| source.error(e))?;
        if values.insert(key.clone(), value).is_some() {
            return Err(source.error(format!("Duplicate parameter {key}")));
        }
    }
    Ok(Entry {
        source: source.clone(),
        values,
    })
}
