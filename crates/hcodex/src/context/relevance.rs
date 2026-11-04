use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::storage::CodegraphReader;

const MAX_SCAN_BYTES: u64 = 64 * 1024;
const MAX_QUERY_TOKENS: usize = 24;
const ALLOWED_EXTENSIONS: &[&str] = &["rs", "py", "ts", "tsx", "js", "jsx", "toml", "sql"];
const KOREAN_DOMAIN_KEYWORDS: &[(&str, &str)] = &[
    ("자동완성", "completion"),
    ("렌더링", "render"),
    ("라우팅", "routing"),
    ("스토리지", "storage"),
    ("토폴로지", "planner"),
    ("정산", "verifier"),
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelevanceCandidate {
    pub path: PathBuf,
    pub symbols: Vec<String>,
    pub score: u32,
    pub rationale: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelevanceContext {
    pub files: Vec<PathBuf>,
    pub symbols: Vec<String>,
    pub candidates: Vec<RelevanceCandidate>,
    pub selection_reason: String,
    pub codegraph_used: bool,
}

#[derive(Default)]
struct Score {
    points: u32,
    symbols: BTreeSet<String>,
    rationale: BTreeSet<String>,
}

pub fn discover_relevance(
    root: &Path,
    prompt: &str,
    max_files: usize,
) -> crate::Result<RelevanceContext> {
    let root = root.canonicalize()?;
    if !root.is_dir() {
        return Err("relevance root must be a directory".into());
    }
    let limit = max_files.clamp(1, 20);
    let tokens = query_tokens(prompt)?;
    let inventory = source_inventory(&root)?;
    let mut scores: BTreeMap<PathBuf, Score> = BTreeMap::new();
    score_explicit_paths(&root, prompt, &mut scores)?;
    let codegraph_used = score_codegraph(&root, &tokens, &mut scores)?;
    score_inventory(&root, &inventory, &tokens, &mut scores);
    let matched = !scores.is_empty();
    if !matched {
        for path in inventory.iter().take(limit) {
            scores.insert(
                path.clone(),
                Score {
                    points: 1,
                    symbols: BTreeSet::new(),
                    rationale: BTreeSet::from([
                        "workspace inventory fallback; no semantic match".to_owned()
                    ]),
                },
            );
        }
    }
    let mut candidates = scores
        .into_iter()
        .map(|(path, score)| RelevanceCandidate {
            path,
            symbols: score.symbols.into_iter().collect(),
            score: score.points,
            rationale: score.rationale.into_iter().collect(),
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.path.cmp(&right.path))
    });
    candidates.truncate(limit);
    let files = candidates
        .iter()
        .map(|candidate| candidate.path.clone())
        .collect();
    let symbols = candidates
        .iter()
        .flat_map(|candidate| candidate.symbols.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(RelevanceContext {
        files,
        symbols,
        candidates,
        selection_reason: if matched {
            "ranked explicit paths, CodeGraph symbols, path tokens, and bounded source matches"
                .to_owned()
        } else {
            "workspace_inventory_fallback_no_semantic_match".to_owned()
        },
        codegraph_used,
    })
}

fn query_tokens(prompt: &str) -> crate::Result<Vec<String>> {
    let matcher = Regex::new(r"(?u)[\p{L}\p{N}_./@-]{3,}")?;
    let ignored = [
        "the",
        "and",
        "for",
        "with",
        "from",
        "this",
        "that",
        "into",
        "please",
        "implement",
        "refactor",
        "review",
        "code",
        "file",
        "기능",
        "구현",
        "검토",
        "해주세요",
    ];
    let mut candidates = Vec::new();
    for item in matcher.find_iter(prompt) {
        let token = item
            .as_str()
            .trim_matches(|ch: char| matches!(ch, '@' | '/' | '.' | '-'))
            .to_lowercase();
        if token.len() >= 3 && !ignored.contains(&token.as_str()) {
            candidates.push((item.start(), 0_u8, token));
        }
    }
    for &(korean, english) in KOREAN_DOMAIN_KEYWORDS {
        for (offset, _) in prompt.match_indices(korean) {
            candidates.push((offset, 1_u8, (*english).to_owned()));
        }
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let mut seen = BTreeSet::new();
    let mut ordered = Vec::new();
    for (_, _, token) in candidates {
        if seen.insert(token.clone()) {
            ordered.push(token);
            if ordered.len() >= MAX_QUERY_TOKENS {
                break;
            }
        }
    }
    Ok(ordered)
}

fn score_explicit_paths(
    root: &Path,
    prompt: &str,
    scores: &mut BTreeMap<PathBuf, Score>,
) -> crate::Result<()> {
    for relative in super::source::explicit_paths(root, prompt)? {
        let reason = format!("explicit @{} path mention", relative.display());
        let score = scores.entry(relative).or_default();
        score.points = score.points.saturating_add(1_000);
        score.rationale.insert(reason);
    }
    Ok(())
}

fn score_codegraph(
    root: &Path,
    tokens: &[String],
    scores: &mut BTreeMap<PathBuf, Score>,
) -> crate::Result<bool> {
    let database = root.join(".codegraph/codegraph.db");
    if !database.is_file() || tokens.is_empty() {
        return Ok(false);
    }
    let Ok(reader) = CodegraphReader::open(&database) else {
        return Ok(false);
    };
    for token in tokens {
        let Ok(nodes) = reader.find_symbol(token) else {
            continue;
        };
        for node in nodes {
            let raw = Path::new(&node.file_path);
            let candidate = if raw.is_absolute() {
                raw.to_path_buf()
            } else {
                root.join(raw)
            };
            let Ok(canonical) = candidate.canonicalize() else {
                continue;
            };
            if !canonical.starts_with(root) || !canonical.is_file() {
                continue;
            }
            let Ok(relative) = canonical.strip_prefix(root).map(Path::to_path_buf) else {
                continue;
            };
            if relative
                .components()
                .any(|part| part.as_os_str() == ".hcodex")
            {
                continue;
            }
            let score = scores.entry(relative).or_default();
            score.points = score.points.saturating_add(250);
            score.symbols.insert(node.qualified_name.clone());
            score
                .rationale
                .insert(format!("CodeGraph symbol match for '{token}'"));
            if !node.callers.is_empty() || !node.callees.is_empty() {
                score.points = score.points.saturating_add(25);
                score
                    .rationale
                    .insert("CodeGraph one-hop relationships available".to_owned());
            }
        }
    }
    Ok(true)
}

fn score_inventory(
    root: &Path,
    inventory: &[PathBuf],
    tokens: &[String],
    scores: &mut BTreeMap<PathBuf, Score>,
) {
    for path in inventory {
        let path_text = path.to_string_lossy().to_lowercase();
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_lowercase();
        let source = fs::read_to_string(root.join(path))
            .unwrap_or_default()
            .to_lowercase();
        let mut path_matches = 0_u32;
        let mut stem_matches = 0_u32;
        let mut source_matches = 0_u32;
        for token in tokens {
            if path_text.contains(token) {
                path_matches = path_matches.saturating_add(1);
            }
            if !stem.is_empty() && (stem == *token || token.ends_with(&format!("/{stem}"))) {
                stem_matches = stem_matches.saturating_add(1);
            }
            if source.contains(token) {
                source_matches = source_matches.saturating_add(1);
            }
        }
        if path_matches == 0 && source_matches == 0 {
            continue;
        }
        let score = scores.entry(path.clone()).or_default();
        score.points = score
            .points
            .saturating_add(path_matches.saturating_mul(40))
            .saturating_add(stem_matches.saturating_mul(160))
            .saturating_add(source_matches.saturating_mul(8));
        if path_matches > 0 {
            score
                .rationale
                .insert(format!("{path_matches} prompt token path match(es)"));
        }
        if source_matches > 0 {
            score
                .rationale
                .insert(format!("{source_matches} bounded source token match(es)"));
        }
    }
}

fn source_inventory(root: &Path) -> crate::Result<Vec<PathBuf>> {
    let mut files = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            !matches!(
                entry.file_name().to_str(),
                Some(
                    ".git"
                        | ".codegraph"
                        | ".venv"
                        | ".agents"
                        | ".omx"
                        | ".codex"
                        | ".hcodex"
                        | "target"
                        | "node_modules"
                )
            )
        })
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| {
            entry
                .metadata()
                .is_ok_and(|metadata| metadata.len() <= MAX_SCAN_BYTES)
        })
        .filter(|entry| {
            entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| ALLOWED_EXTENSIONS.contains(&extension))
        })
        .filter_map(|entry| entry.path().strip_prefix(root).ok().map(Path::to_path_buf))
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests;
