use std::path::{Component, Path, PathBuf};

use regex::Regex;

use crate::Result;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SourceReference {
    pub path: PathBuf,
    pub lines: Option<(usize, usize)>,
}

pub(super) fn parse_reference(raw: &str) -> Result<SourceReference> {
    let (path, lines) = if let Some((path, suffix)) = raw.rsplit_once(':') {
        if suffix.starts_with(|character: char| character.is_ascii_digit() || character == 'L') {
            let (start, end) = suffix.split_once('-').unwrap_or((suffix, suffix));
            let start = start.trim_start_matches('L').parse::<usize>()?;
            let end = end.trim_start_matches('L').parse::<usize>()?;
            if start == 0 || end < start {
                return Err(format!("invalid source line range: {raw}").into());
            }
            (path, Some((start, end)))
        } else {
            (raw, None)
        }
    } else {
        (raw, None)
    };
    Ok(SourceReference {
        path: validated_path(path)?,
        lines,
    })
}

pub(super) fn validated_path(raw: &str) -> Result<PathBuf> {
    let path = Path::new(raw);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("unsafe source context path: {}", path.display()).into());
    }
    Ok(path.to_path_buf())
}

pub(super) fn file_mentions(text: &str) -> Result<Vec<SourceReference>> {
    let matcher = Regex::new(
        r#"(?:^|[\s(])@("(?:\\.|[^"\\\r\n])+"|'[^'\r\n]+'|[^\s@`"'<>()\[\]{},;]+)(?::(L?\d+(?:-L?\d+)?))?"#,
    )?;
    let quoted_start = Regex::new(r#"(?:^|[\s(])@["']"#)?;
    let mut references = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let mut complete = Vec::new();
        for capture in matcher.captures_iter(line) {
            if let Some(span) = capture.get(0) {
                complete.push((span.start(), span.end()));
            }
            let raw = &capture[1];
            let path = if raw.starts_with('"') {
                serde_json::from_str::<String>(raw)?
            } else if raw.starts_with('\'') {
                raw[1..raw.len() - 1].to_owned()
            } else {
                raw.trim_end_matches(['.', ':']).to_owned()
            };
            let value = capture
                .get(2)
                .map_or(path.clone(), |range| format!("{path}:{}", range.as_str()));
            let reference =
                if (raw.starts_with('"') || raw.starts_with('\'')) && capture.get(2).is_none() {
                    SourceReference {
                        path: validated_path(&path)?,
                        lines: None,
                    }
                } else {
                    parse_reference(&value)?
                };
            if !references.contains(&reference) {
                references.push(reference);
            }
            if references.len() > 32 {
                return Err("too many explicit source references (maximum 32)".into());
            }
        }
        if quoted_start.find_iter(line).any(|start| {
            let quote = start.end() - 1;
            !complete
                .iter()
                .any(|(begin, end)| *begin <= quote && quote < *end)
        }) {
            return Err("unterminated quoted source reference".into());
        }
    }
    Ok(references)
}

pub(super) fn diff_references(text: &str) -> Result<Vec<SourceReference>> {
    let hunk = Regex::new(r"^@@\s+-\d+(?:,\d+)?\s+\+(\d+)(?:,(\d+))?\s+@@")?;
    let mut path = None;
    let mut references = Vec::new();
    for line in text.lines() {
        if let Some(raw) = line.strip_prefix("+++ ") {
            let raw = raw.split('\t').next().unwrap_or_default().trim();
            let raw = if raw.starts_with('"') {
                serde_json::from_str::<String>(raw)?
            } else {
                raw.to_owned()
            };
            path = if raw == "/dev/null" {
                None
            } else {
                Some(validated_path(raw.strip_prefix("b/").unwrap_or(&raw))?)
            };
        } else if let Some(capture) = hunk.captures(line)
            && let Some(path) = &path
        {
            let start = capture[1].parse::<usize>()?.max(1);
            let count = capture
                .get(2)
                .map(|value| value.as_str().parse::<usize>())
                .transpose()?
                .unwrap_or(1)
                .max(1);
            let end = start
                .checked_add(count - 1)
                .ok_or("diff line range overflow")?;
            let reference = SourceReference {
                path: path.clone(),
                lines: Some((start, end)),
            };
            if !references.contains(&reference) {
                references.push(reference);
            }
            if references.len() > 32 {
                return Err("too many source diff hunks (maximum 32)".into());
            }
        }
    }
    Ok(references)
}

pub(super) fn inline_symbols(text: &str) -> Result<Vec<String>> {
    let matcher = Regex::new(r"\b(?:fn|def|class|struct|enum|trait)\s+([\p{L}_][\p{L}\p{N}_]*)")?;
    Ok(matcher
        .captures_iter(text)
        .map(|capture| capture[1].to_owned())
        .take(12)
        .collect())
}
