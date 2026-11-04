use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::storage::{CodegraphNode, CodegraphReader};

mod references;
use references::{SourceReference, diff_references, file_mentions, inline_symbols, validated_path};

const MAX_SOURCE_READ_BYTES: u64 = 4 * 1024 * 1024;
const MAX_MENTION_BYTES: usize = 8_000;
const MAX_SOURCE_LINES: usize = 100_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceExcerpt {
    pub path: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
    pub matched_symbols: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroundedContext {
    pub excerpts: Vec<SourceExcerpt>,
    pub markdown: String,
    pub truncated: bool,
}

#[derive(Clone, Debug)]
pub struct SourceContextBuilder {
    root: PathBuf,
}

impl SourceContextBuilder {
    pub fn new(root: &Path) -> Result<Self> {
        let root = root.canonicalize()?;
        if !root.is_dir() {
            return Err("source context workspace must be a directory".into());
        }
        Ok(Self { root })
    }

    pub fn build(
        &self,
        files: &[PathBuf],
        symbols: &[String],
        max_bytes: usize,
    ) -> Result<GroundedContext> {
        self.build_with_text(files, symbols, "", max_bytes)
    }

    /// Resolve actual user-selected attachments for adapters and evidence scopes.
    pub fn referenced_paths(&self, text: &str) -> Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for reference in file_mentions(text)? {
            let path = self.resolve_inside(&reference.path)?.0;
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        for reference in diff_references(text)? {
            if !self.root.join(&reference.path).exists() {
                continue;
            }
            let path = self.resolve_inside(&reference.path)?.0;
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        Ok(paths)
    }

    pub(crate) fn normalized_files(&self, files: &[PathBuf]) -> Result<Vec<PathBuf>> {
        files
            .iter()
            .map(|path| self.resolve_inside(path).map(|resolved| resolved.0))
            .collect()
    }

    /// Ground user attachments and diff/symbol selections in actual disk bytes.
    pub fn build_with_text(
        &self,
        files: &[PathBuf],
        symbols: &[String],
        inline_text: &str,
        max_bytes: usize,
    ) -> Result<GroundedContext> {
        let mut mentions = file_mentions(inline_text)?;
        for reference in &mut mentions {
            reference.path = self.resolve_inside(&reference.path)?.0;
        }
        let mut selected = mentions.clone();
        let mut diff_ranges = Vec::new();
        let mut absent_diff_paths = Vec::new();
        for mut reference in diff_references(inline_text)? {
            if !self.root.join(&reference.path).exists() {
                absent_diff_paths.push(reference.path);
                continue;
            }
            reference.path = self.resolve_inside(&reference.path)?.0;
            diff_ranges.push(reference.clone());
            if !selected.contains(&reference) {
                selected.push(reference);
            }
        }
        for path in files {
            let reference = SourceReference {
                path: validated_path(&path.to_string_lossy())?,
                lines: None,
            };
            if !selected.iter().any(|current| {
                current.path == reference.path
                    && (current.lines.is_some() || current.lines == reference.lines)
            }) {
                selected.push(reference);
            }
        }
        // Resolve explicit paths before looking at the index or running a provider.
        for reference in &mut selected {
            reference.path = self.resolve_inside(&reference.path)?.0;
        }
        let explicit_paths: BTreeSet<PathBuf> = selected
            .iter()
            .map(|reference| reference.path.clone())
            .collect();
        let explicit_symbols: BTreeSet<String> = symbols
            .iter()
            .map(|symbol| symbol.trim().to_owned())
            .filter(|symbol| !symbol.is_empty())
            .collect();
        let mut requested_symbols = explicit_symbols.clone();
        requested_symbols.extend(inline_symbols(inline_text)?);
        let mut graph_nodes = Vec::new();
        let database = self.root.join(".codegraph/codegraph.db");
        if database.is_file() {
            let reader = CodegraphReader::open(&database)?;
            for symbol in requested_symbols.iter().take(12) {
                for node in reader.find_symbol(symbol)? {
                    if graph_nodes
                        .iter()
                        .all(|current: &CodegraphNode| current.id != node.id)
                    {
                        graph_nodes.push(node);
                    }
                }
            }
            for reference in &selected {
                let has_symbol_in_file = !explicit_symbols.is_empty()
                    && reference.lines.is_none()
                    && graph_nodes.iter().any(|node| {
                        let path = Path::new(&node.file_path);
                        let relative = path.strip_prefix(&self.root).unwrap_or(path);
                        relative == reference.path
                    });
                if has_symbol_in_file {
                    continue;
                }
                for node in
                    reader.find_file_symbols(&reference.path.to_string_lossy(), reference.lines)?
                {
                    if graph_nodes
                        .iter()
                        .all(|current: &CodegraphNode| current.id != node.id)
                    {
                        graph_nodes.push(node);
                    }
                }
            }
            let neighbors = graph_nodes
                .iter()
                .flat_map(|node| node.callers.iter().chain(&node.callees))
                .take(24)
                .cloned()
                .collect::<BTreeSet<_>>();
            for symbol in neighbors {
                for node in reader.find_symbol(&symbol)? {
                    if graph_nodes.iter().all(|current| current.id != node.id) {
                        graph_nodes.push(node);
                    }
                }
            }
        }
        let mut indexed_ranges: BTreeMap<PathBuf, Vec<(usize, usize)>> = BTreeMap::new();
        let mut graph = String::new();
        for path in absent_diff_paths {
            graph.push_str(&format!("Diff target {} is not present on disk; only caller-provided diff content is available.\n", path.display()));
        }
        for node in &graph_nodes {
            let path = Path::new(&node.file_path);
            let relative = if path.is_absolute() {
                path.strip_prefix(&self.root)?.to_path_buf()
            } else {
                path.to_path_buf()
            };
            let (relative, _) = self.resolve_inside(&relative)?;
            if node.start_line <= 0 || node.end_line < node.start_line {
                continue;
            }
            if explicit_paths.is_empty() || explicit_paths.contains(&relative) {
                indexed_ranges.entry(relative.clone()).or_default().push((
                    usize::try_from(node.start_line)?,
                    usize::try_from(node.end_line)?,
                ));
                if !selected.iter().any(|current| current.path == relative) {
                    selected.push(SourceReference {
                        path: relative,
                        lines: None,
                    });
                }
            }
            graph.push_str(&format!(
                "Symbol {} {}:{}-{} · callers={:?} · callees={:?}\n",
                node.qualified_name,
                node.file_path,
                node.start_line,
                node.end_line,
                node.callers,
                node.callees
            ));
        }
        let mut markdown = String::new();
        let mut truncated = false;
        append_bounded(
            &mut markdown,
            "=== GROUNDED WORKSPACE SOURCE ===\n",
            max_bytes,
            &mut truncated,
        );
        let graph_budget = (max_bytes / 4).min(8_000);
        let metadata_limit = graph_budget.max(markdown.len());
        append_bounded(&mut markdown, &graph, metadata_limit, &mut truncated);
        let mut excerpts = Vec::new();
        for (position, reference) in selected.iter().take(32).enumerate() {
            let (relative, absolute) = self.resolve_inside(&reference.path)?;
            let (source, read_truncated) = read_bounded_source(&absolute)?;
            truncated |= read_truncated;
            let lines = source.lines().take(MAX_SOURCE_LINES).collect::<Vec<_>>();
            truncated |= source.lines().count() > MAX_SOURCE_LINES;
            let mut matched = requested_symbols
                .iter()
                .filter_map(|symbol| {
                    let leaf = symbol.rsplit(['.', ':']).next().unwrap_or(symbol);
                    source.contains(leaf).then(|| leaf.to_owned())
                })
                .collect::<Vec<_>>();
            matched.extend(source_symbols_mentioned(&source, inline_text)?);
            matched.sort();
            matched.dedup();
            let indexed = indexed_ranges.get(&relative);
            let expand_diff = diff_ranges.contains(reference)
                && indexed.is_some()
                && !mentions.contains(reference);
            let wanted = if let Some((start, end)) = reference.lines.filter(|_| !expand_diff) {
                if start > lines.len() {
                    return Err(format!(
                        "source line {start} is outside {} ({} lines)",
                        relative.display(),
                        lines.len()
                    )
                    .into());
                }
                truncated |= end > lines.len();
                (start..=end.min(lines.len())).collect::<BTreeSet<_>>()
            } else if let Some(ranges) = indexed {
                let min_start = ranges.iter().map(|(start, _)| *start).min().unwrap_or(1);
                let max_end = ranges
                    .iter()
                    .map(|(_, end)| (*end).min(lines.len()))
                    .max()
                    .unwrap_or(lines.len());
                if explicit_symbols.is_empty() && reference.lines.is_none() {
                    let start = if min_start <= 80 { 1 } else { min_start };
                    let end = if min_start <= 80 {
                        lines.len()
                    } else {
                        max_end
                    };
                    (start..=end).collect()
                } else {
                    ranges
                        .iter()
                        .flat_map(|(start, end)| *start..=(*end).min(lines.len()))
                        .filter(|line| *line > 0 && *line <= lines.len())
                        .collect()
                }
            } else {
                select_lines(&lines, &matched)
            };
            let header = format!("\n### File `{}`\n```text\n", relative.display());
            let footer = "\n```\n";
            let available = max_bytes
                .saturating_sub(markdown.len())
                .saturating_sub(header.len() + footer.len());
            let share = available / selected.len().min(32).saturating_sub(position).max(1);
            let file_budget = if mentions
                .iter()
                .any(|mention| mention.path == reference.path)
            {
                share.min(MAX_MENTION_BYTES)
            } else {
                share
            };
            let mut content = String::new();
            let mut included = Vec::new();
            for line_number in &wanted {
                let formatted = format!("{line_number:>6} | {}\n", lines[*line_number - 1]);
                if content.len() + formatted.len() > file_budget {
                    truncated = true;
                    break;
                }
                content.push_str(&formatted);
                included.push(*line_number);
            }
            if included.is_empty() && !wanted.is_empty() {
                truncated = true;
                continue;
            }
            if markdown.len() + header.len() + content.len() + footer.len() > max_bytes {
                truncated = true;
                break;
            }
            markdown.push_str(&header);
            markdown.push_str(&content);
            markdown.push_str(footer);
            excerpts.push(SourceExcerpt {
                path: relative,
                start_line: included.first().copied().unwrap_or(1),
                end_line: included.last().copied().unwrap_or(0),
                content,
                matched_symbols: matched,
            });
        }
        if selected.len() > 32 {
            truncated = true;
        }
        if truncated {
            let notice = "\n[Source context truncated by byte/read budget; omitted content was not inspected.]\n";
            if markdown.len() + notice.len() <= max_bytes {
                markdown.push_str(notice);
            }
        }
        Ok(GroundedContext {
            excerpts,
            markdown,
            truncated,
        })
    }

    fn resolve_inside(&self, requested: &Path) -> Result<(PathBuf, PathBuf)> {
        let requested = validated_path(&requested.to_string_lossy())?;
        if requested
            .components()
            .any(|part| part.as_os_str() == ".hcodex")
        {
            return Err("private recovery state is not source context".into());
        }
        let absolute = self.root.join(&requested).canonicalize().map_err(|error| {
            format!(
                "source reference {} unavailable: {error}",
                requested.display()
            )
        })?;
        if !absolute.starts_with(&self.root) {
            return Err(
                format!("source context escapes workspace: {}", requested.display()).into(),
            );
        }
        if absolute
            .strip_prefix(&self.root)?
            .components()
            .any(|part| part.as_os_str() == ".hcodex")
        {
            return Err("source context resolves to private recovery state".into());
        }
        if !absolute.is_file() {
            return Err(
                format!("source context path is not a file: {}", requested.display()).into(),
            );
        }
        Ok((absolute.strip_prefix(&self.root)?.to_path_buf(), absolute))
    }
}

pub(super) fn explicit_paths(root: &Path, text: &str) -> Result<Vec<PathBuf>> {
    let builder = SourceContextBuilder::new(root)?;
    builder.referenced_paths(text)
}

fn append_bounded(output: &mut String, text: &str, budget: usize, truncated: &mut bool) {
    let mut end = text.len().min(budget.saturating_sub(output.len()));
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    output.push_str(&text[..end]);
    *truncated |= end != text.len();
}

fn read_bounded_source(path: &Path) -> Result<(String, bool)> {
    let file = fs::File::open(path)?;
    let truncated = file.metadata()?.len() > MAX_SOURCE_READ_BYTES;
    let mut bytes = Vec::new();
    file.take(MAX_SOURCE_READ_BYTES).read_to_end(&mut bytes)?;
    let valid = match std::str::from_utf8(&bytes) {
        Ok(_) => bytes.len(),
        Err(error) if truncated && error.error_len().is_none() => error.valid_up_to(),
        Err(error) => {
            return Err(format!(
                "source reference is not UTF-8 text: {}: {error}",
                path.display()
            )
            .into());
        }
    };
    bytes.truncate(valid);
    Ok((String::from_utf8(bytes)?, truncated))
}

fn source_symbols_mentioned(source: &str, text: &str) -> Result<Vec<String>> {
    let words = Regex::new(r"[\p{L}_][\p{L}\p{N}_]*")?
        .find_iter(text)
        .map(|item| item.as_str())
        .collect::<BTreeSet<_>>();
    let definitions =
        Regex::new(r"\b(?:fn|def|class|struct|enum|trait)\s+([\p{L}_][\p{L}\p{N}_]*)")?;
    Ok(definitions
        .captures_iter(source)
        .map(|capture| capture[1].to_owned())
        .filter(|name| words.contains(name.as_str()))
        .take(12)
        .collect())
}

fn select_lines(lines: &[&str], symbols: &[String]) -> BTreeSet<usize> {
    if symbols.is_empty() {
        return (1..=lines.len()).collect();
    }
    let mut selected = BTreeSet::new();
    for (index, line) in lines.iter().enumerate() {
        if symbols.iter().any(|symbol| line.contains(symbol)) {
            let indentation = line.len() - line.trim_start().len();
            let definition = ["fn ", "def ", "class ", "struct ", "enum ", "trait "]
                .iter()
                .any(|prefix| line.contains(prefix));
            let end = if definition {
                lines
                    .iter()
                    .enumerate()
                    .skip(index + 1)
                    .find(|(_, next)| {
                        let trimmed = next.trim_start();
                        let next_indent = next.len() - trimmed.len();
                        next_indent <= indentation
                            && [
                                "fn ",
                                "pub fn ",
                                "async fn ",
                                "def ",
                                "class ",
                                "struct ",
                                "enum ",
                                "trait ",
                            ]
                            .iter()
                            .any(|prefix| trimmed.starts_with(prefix))
                    })
                    .map_or(lines.len(), |(next, _)| next)
            } else {
                (index + 4).min(lines.len())
            };
            selected.extend(index.saturating_sub(3) + 1..=end);
        }
    }
    selected
}
#[cfg(test)]
mod tests;
