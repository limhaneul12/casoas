use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::Result;

const DEFAULT_SKILL: &str = "codex-heterarchy";
const CORE_SKILLS: &[&str] = &[
    DEFAULT_SKILL,
    "codegraph-operator",
    "codebase-refactor-guardian",
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillScope {
    Workspace,
    Global,
    Gemini,
    Plugin,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub scope: SkillScope,
}

#[derive(Clone, Debug)]
pub struct SkillRegistry {
    skills: BTreeMap<String, SkillEntry>,
    active: Vec<String>,
}

impl SkillRegistry {
    pub fn discover(workspace: &Path, home: Option<&Path>) -> Result<Self> {
        let workspace = workspace.canonicalize()?;
        let home = match home {
            Some(path) => path.to_path_buf(),
            None => std::env::var_os("HCODEX_HOME")
                .or_else(|| std::env::var_os("HOME"))
                .map(PathBuf::from)
                .ok_or("HOME is not available and no home path was supplied")?,
        };
        let roots = [
            (workspace.join(".agents"), SkillScope::Workspace),
            (workspace.join(".codex/skills"), SkillScope::Workspace),
            (workspace.join("skills"), SkillScope::Workspace),
            (workspace.join("plugin"), SkillScope::Plugin),
            (home.join(".codex/skills"), SkillScope::Global),
            (home.join(".agents/skills"), SkillScope::Global),
            (home.join(".codex/plugins/cache"), SkillScope::Plugin),
            (home.join(".gemini/config/skills"), SkillScope::Gemini),
            (home.join(".gemini/config/plugins"), SkillScope::Plugin),
        ];
        let mut skills = BTreeMap::new();
        for (root, scope) in roots {
            if !root.is_dir() {
                continue;
            }
            for entry in WalkDir::new(&root)
                // User-installed skills may be symlinked into these explicit roots.
                // WalkDir detects cycles; deterministic traversal preserves precedence.
                .follow_links(true)
                .sort_by_file_name()
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if entry.file_type().is_file() && entry.file_name() == "SKILL.md" {
                    let body = fs::read_to_string(entry.path())?;
                    let fallback = entry
                        .path()
                        .parent()
                        .and_then(Path::file_name)
                        .and_then(|name| name.to_str())
                        .unwrap_or("unnamed-skill");
                    let (name, description) = parse_frontmatter(&body, fallback);
                    skills.entry(name.clone()).or_insert_with(|| SkillEntry {
                        name,
                        description,
                        path: entry.path().to_path_buf(),
                        scope: scope.clone(),
                    });
                }
            }
        }
        let active = skills
            .contains_key(DEFAULT_SKILL)
            .then(|| DEFAULT_SKILL.to_owned())
            .into_iter()
            .collect();
        Ok(Self { skills, active })
    }

    pub fn skills(&self) -> &BTreeMap<String, SkillEntry> {
        &self.skills
    }

    pub fn active(&self) -> &[String] {
        &self.active
    }

    pub fn activate(&mut self, name: &str) -> Result<()> {
        let key =
            resolve_key(&self.skills, name).ok_or_else(|| format!("unknown skill: {name}"))?;
        if !self.active.contains(&key) {
            self.active.push(key);
        }
        Ok(())
    }

    pub fn deactivate(&mut self, name: &str) {
        if let Some(key) = resolve_key(&self.skills, name) {
            self.active.retain(|active| active != &key);
        }
    }

    /// Restore the historical core preset, reporting unavailable members.
    pub(crate) fn activate_core(&mut self) -> Vec<String> {
        self.active.clear();
        let mut missing = Vec::new();
        for &name in CORE_SKILLS {
            if self.skills.contains_key(name) {
                self.active.push(name.to_owned());
            } else {
                missing.push(name.to_owned());
            }
        }
        missing
    }

    pub fn extract_mentions(&mut self, input: &str) -> (Vec<String>, String) {
        self.extract_mentions_where(input, |_| true)
    }

    /// Extract registered top-level tokens without rewriting quoted payloads.
    /// The caller may reserve its own command vocabulary without coupling this
    /// registry to a particular UI or command parser.
    pub(crate) fn extract_mentions_where(
        &mut self,
        input: &str,
        eligible: impl Fn(&str) -> bool,
    ) -> (Vec<String>, String) {
        let mut activated = Vec::new();
        let mut removed = Vec::new();
        for range in top_level_tokens(input) {
            let token = &input[range.clone()];
            if !eligible(token) {
                continue;
            }
            let candidate = token.strip_prefix('$').or_else(|| token.strip_prefix('/'));
            let resolved = candidate.and_then(|name| resolve_key(&self.skills, name));
            if let Some(key) = resolved {
                if !self.active.contains(&key) {
                    self.active.push(key.clone());
                }
                activated.push(key);
                removed.push(range);
            }
        }
        if removed.is_empty() {
            return (activated, input.to_owned());
        }
        let mut remaining = String::with_capacity(input.len());
        let mut cursor = 0;
        for range in removed {
            remaining.push_str(&input[cursor..range.start]);
            cursor = range.end;
        }
        remaining.push_str(&input[cursor..]);
        (activated, remaining.trim().to_owned())
    }

    pub fn active_context(&self, max_bytes_per_skill: usize) -> Result<String> {
        let mut blocks = Vec::new();
        for name in &self.active {
            let Some(entry) = self.skills.get(name) else {
                continue;
            };
            let body = fs::read_to_string(&entry.path)?;
            let excerpt = truncate_utf8(&body, max_bytes_per_skill);
            blocks.push(format!(
                "### Skill: `{}` ({:?})\n{}",
                entry.name, entry.scope, excerpt
            ));
            if excerpt.len() < body.len() {
                blocks.push(format!(
                    "[Skill `{}` truncated at {} of {} UTF-8 bytes]",
                    entry.name,
                    excerpt.len(),
                    body.len()
                ));
            }
        }
        if blocks.is_empty() {
            return Ok(String::new());
        }
        Ok(format!(
            "=== ACTIVE REGISTERED SKILLS ({}) ===\n\n{}",
            self.active.join(", "),
            blocks.join("\n\n")
        ))
    }
}

fn top_level_tokens(input: &str) -> Vec<std::ops::Range<usize>> {
    let mut tokens = Vec::new();
    let mut start = None;
    let mut quote = None;
    let mut code_ticks = 0;
    let mut escaped = false;
    let mut offset = 0;
    while offset < input.len() {
        let Some(character) = input[offset..].chars().next() else {
            break;
        };
        if code_ticks > 0 {
            if character == '`' {
                let count = input.as_bytes()[offset..]
                    .iter()
                    .take_while(|&&byte| byte == b'`')
                    .count();
                if count == code_ticks {
                    code_ticks = 0;
                }
                offset += count;
            } else {
                offset += character.len_utf8();
            }
            continue;
        }
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == delimiter {
                quote = None;
            }
            offset += character.len_utf8();
            continue;
        }
        if character.is_whitespace() {
            if let Some(begin) = start.take() {
                tokens.push(begin..offset);
            }
            offset += character.len_utf8();
            continue;
        }
        let begin = *start.get_or_insert(offset);
        if character == '`' {
            code_ticks = input.as_bytes()[offset..]
                .iter()
                .take_while(|&&byte| byte == b'`')
                .count();
            offset += code_ticks;
            continue;
        }
        if character == '"' || character == '\'' && begin == offset {
            quote = Some(character);
        }
        offset += character.len_utf8();
    }
    if let Some(begin) = start {
        tokens.push(begin..input.len());
    }
    tokens
}

fn resolve_key(skills: &BTreeMap<String, SkillEntry>, raw: &str) -> Option<String> {
    let normalized = raw.trim().trim_start_matches(['$', '/']).to_lowercase();
    if skills.contains_key(&normalized) {
        return Some(normalized);
    }
    normalized
        .split_once(':')
        .and_then(|(_, skill)| skills.contains_key(skill).then(|| skill.to_owned()))
}

fn parse_frontmatter(body: &str, fallback: &str) -> (String, String) {
    let mut name = fallback.to_lowercase();
    let mut description_parts = Vec::new();
    let normalized = body.strip_prefix('\u{feff}').unwrap_or(body);
    let frontmatter = normalized
        .strip_prefix("---\n")
        .or_else(|| normalized.strip_prefix("---\r\n"))
        .and_then(|rest| rest.split_once("\n---"));
    if let Some((header, _)) = frontmatter {
        let mut in_description = false;
        for line in header.lines() {
            let trimmed = line.trim();
            let indented = line.starts_with(' ') || line.starts_with('\t');
            if in_description && indented {
                if !trimmed.is_empty() {
                    description_parts.push(trimmed.trim_matches(['\"', '\'']).to_owned());
                }
                continue;
            }
            in_description = false;
            if let Some(value) = trimmed.strip_prefix("name:") {
                let value = value.trim().trim_matches(['\"', '\'']).to_lowercase();
                if !value.is_empty() {
                    name = value;
                }
            } else if let Some(value) = trimmed.strip_prefix("description:") {
                let value = value.trim().trim_matches(['\"', '\'']);
                if !matches!(value, "" | ">" | "|" | ">-" | "|-" | ">+" | "|+") {
                    description_parts.push(value.to_owned());
                }
                in_description = true;
            }
        }
    }
    (name, description_parts.join(" "))
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

#[cfg(test)]
mod tests;
