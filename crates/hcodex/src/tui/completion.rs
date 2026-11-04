//! Slash, skill, and workspace-file completion catalog.

use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use walkdir::{DirEntry, WalkDir};

use crate::Result;
use crate::context::{SkillRegistry, SkillScope};

const COMMANDS: &[&str] = &[
    "/dag-ulw",
    "/dag-run",
    "/dag",
    "/dag-plan",
    "/dag-mode",
    "/dag-exit",
    "/omh",
    "/board",
    "/council",
    "/votes",
    "/recovery",
    "/todo",
    "/plan",
    "/prompts",
    "/promt",
    "/trace",
    "/prompt-trace",
    "/back",
    "/prev",
    "/models",
    "/lanes",
    "/model",
    "/use",
    "/ask",
    "/prompt",
    "/sonnet",
    "/opus",
    "/argon",
    "/flash",
    "/skills",
    "/skill",
    "/effect",
    "/effort",
    "/reasoning",
    "/dag-effect",
    "/status",
    "/jobs",
    "/inbox",
    "/compact",
    "/permissions",
    "/approvals",
    "/resume",
    "/triad",
    "/triad-review",
    "/diff",
    "/review",
    "/clear",
    "/help",
    "/theme",
    "/quit",
    "/exit",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionKind {
    Command,
    Skill,
    File,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletionItem {
    pub kind: CompletionKind,
    pub value: String,
    pub detail: String,
    pub display_label: String,
    pub badge: String,
    pub replace_start: usize,
    pub submit_on_enter: bool,
}

#[derive(Clone, Debug)]
struct CatalogEntry {
    item: CompletionItem,
    keywords: String,
}

#[derive(Clone, Debug, Default)]
pub struct TuiCatalog {
    entries: Vec<CatalogEntry>,
    git_branch: Option<String>,
}

impl TuiCatalog {
    pub(crate) fn is_exact_command(&self, value: &str) -> bool {
        self.entries.iter().any(|entry| {
            entry.item.kind == CompletionKind::Command
                && entry.item.submit_on_enter
                && entry.item.value.eq_ignore_ascii_case(value.trim())
        })
    }
    pub(crate) fn is_skill_reference(&self, value: &str) -> bool {
        self.entries.iter().any(|entry| {
            entry.item.kind == CompletionKind::Skill
                && skill_key(&entry.item.value) == skill_key(value)
        })
    }

    #[cfg(test)]
    pub(crate) fn command_catalog() -> Self {
        Self {
            entries: command_entries(),
            git_branch: None,
        }
    }
    pub fn discover(workspace: &Path) -> Result<Self> {
        let home = std::env::var_os("HCODEX_HOME")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from);
        Self::discover_with_home(workspace, home.as_deref())
    }

    fn discover_with_home(workspace: &Path, home: Option<&Path>) -> Result<Self> {
        let mut entries = command_entries();
        let registry = SkillRegistry::discover(workspace, home)?;
        for skill in registry.skills().values() {
            if skill.name.chars().any(char::is_control) {
                continue;
            }
            let plugin = (skill.scope == SkillScope::Plugin)
                .then(|| plugin_name(&skill.path))
                .flatten();
            let value = plugin.as_ref().map_or_else(
                || format!("${}", skill.name),
                |plugin| format!("${plugin}:{}", skill.name),
            );
            entries.push(entry(
                CompletionKind::Skill,
                &value,
                &value,
                &format!("{:?}", skill.scope),
                &skill.description,
                &format!(
                    "스킬 {} {} {}",
                    skill.name,
                    plugin.unwrap_or_default(),
                    core_keywords(&skill.name)
                ),
                false,
            ));
            for command in ["/skill", "/skills"] {
                entries.push(entry(
                    CompletionKind::Command,
                    &format!("{command} {}", skill.name),
                    &value,
                    "SKILL",
                    &skill.description,
                    &format!("스킬 {}", skill.name),
                    true,
                ));
            }
        }
        for value in discover_files(workspace)? {
            entries.push(entry(
                CompletionKind::File,
                &value,
                &value,
                "FILE",
                "workspace file or directory",
                "파일 경로 file path",
                false,
            ));
        }
        Ok(Self {
            entries,
            git_branch: discover_git_branch(workspace),
        })
    }

    pub(crate) fn git_branch(&self) -> Option<&str> {
        self.git_branch.as_deref()
    }

    pub fn complete(&self, input: &str, cursor: usize, limit: usize) -> Vec<CompletionItem> {
        self.complete_with_state(input, cursor, limit, &[], false)
    }

    pub(crate) fn complete_with_state(
        &self,
        input: &str,
        cursor: usize,
        limit: usize,
        active: &[String],
        force_open: bool,
    ) -> Vec<CompletionItem> {
        let mut cursor = cursor.min(input.len());
        while !input.is_char_boundary(cursor) {
            cursor = cursor.saturating_sub(1);
        }
        let prefix = &input[..cursor];
        let token_start = token_start(prefix);
        let token = &prefix[token_start..];
        let contextual = contextual_command(prefix);
        let (file_query, file_suffix) = file_query(token);
        let query = if contextual.is_some() {
            prefix.trim_end().to_lowercase()
        } else {
            file_query.to_lowercase()
        };
        let keyword_query = contextual.map_or_else(
            || query.trim_start_matches(['$', '/']).to_owned(),
            |command| query[command.len()..].trim().to_owned(),
        );
        let prior = &prefix[..token_start];
        let used = prior
            .split_whitespace()
            .filter(|value| value.starts_with('$'))
            .map(skill_key)
            .collect::<BTreeSet<_>>();
        let allow_commands =
            token_start == 0 || prior.split_whitespace().all(|value| value.starts_with('$'));
        if query.is_empty() && !force_open && contextual.is_none() {
            return Vec::new();
        }
        let mut scored = Vec::new();
        for candidate in &self.entries {
            let item = &candidate.item;
            let registered_skill =
                item.kind == CompletionKind::Skill || item.display_label.starts_with('$');
            let key = if registered_skill {
                skill_key(&item.display_label)
            } else {
                skill_key(&item.value)
            };
            if item.kind == CompletionKind::Skill && used.contains(&key) {
                continue;
            }
            let replace_start = if let Some(command) = contextual {
                if item.kind != CompletionKind::Command
                    || !item.value.starts_with(&format!("{command} "))
                {
                    continue;
                }
                0
            } else {
                let plain_len = token.chars().count();
                let file_like_plain = plain_len >= 3
                    && (token.contains('.')
                        || token.contains('/')
                        || item
                            .value
                            .trim_start_matches(['@', '"'])
                            .trim_end_matches('"')
                            .rsplit('/')
                            .next()
                            .is_some_and(|base| {
                                base.to_lowercase().starts_with(&token.to_lowercase())
                            }));
                match token.chars().next() {
                    Some('@') if item.kind == CompletionKind::File => {}
                    Some('$') if item.kind != CompletionKind::Skill => continue,
                    Some('@') if item.kind != CompletionKind::File => continue,
                    Some('/') if item.kind == CompletionKind::File || !allow_commands => continue,
                    Some('/') if token == "/" && item.kind == CompletionKind::Skill => continue,
                    _ if item.kind == CompletionKind::File && !file_like_plain => continue,
                    _ if !token.starts_with(['$', '/', '@'])
                        && token_start > 0
                        && !force_open
                        && (plain_len < 2 || item.kind == CompletionKind::Command) =>
                    {
                        continue;
                    }
                    _ if !allow_commands && item.kind == CompletionKind::Command => continue,
                    _ => {}
                }
                token_start
            };
            let stripped = query.trim_start_matches(['$', '/', '@', '"']);
            let value = item.value.to_lowercase();
            let label = item.display_label.to_lowercase();
            let searchable = format!(
                "{} {}",
                item.detail.to_lowercase(),
                candidate.keywords.to_lowercase()
            );
            let score = if query.is_empty() {
                if value == "/skills" {
                    0
                } else if item.kind == CompletionKind::Skill
                    && active.iter().any(|name| skill_key(name) == key)
                {
                    1
                } else if item.kind == CompletionKind::Command {
                    2
                } else {
                    3
                }
            } else if value == query {
                0
            } else if value.starts_with(&query)
                || label.trim_start_matches(['$', '/']).starts_with(stripped)
                || (item.kind == CompletionKind::Skill && key.starts_with(stripped))
            {
                1
            } else if value.contains(&query) || label.contains(stripped) {
                2
            } else if searchable.contains(&keyword_query) {
                3
            } else if contextual.is_none() {
                match fuzzy_score(&value, &query) {
                    Some(score) => 4 + score,
                    None => continue,
                }
            } else {
                continue;
            };
            let active_rank =
                usize::from(registered_skill && !active.iter().any(|name| skill_key(name) == key));
            let mut completed = item.clone();
            completed.replace_start = replace_start;
            if item.kind == CompletionKind::File {
                completed.value.push_str(file_suffix);
            }
            if registered_skill && active_rank == 0 {
                completed.badge = format!("● ON · {}", item.badge);
            }
            let preset_rank = usize::from(
                registered_skill
                    && contextual.is_some_and(|command| matches!(command, "/skill" | "/skills")),
            );
            scored.push((score, preset_rank, active_rank, completed));
        }
        scored.sort_by(|left, right| {
            (left.0, left.1, left.2, &left.3.value).cmp(&(
                right.0,
                right.1,
                right.2,
                &right.3.value,
            ))
        });
        scored
            .into_iter()
            .take(limit)
            .map(|(_, _, _, item)| item)
            .collect()
    }
}

fn discover_git_branch(workspace: &Path) -> Option<String> {
    let workspace = workspace.canonicalize().ok()?;
    for directory in workspace.ancestors() {
        let marker = directory.join(".git");
        match fs::symlink_metadata(&marker) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return None,
            Ok(_) => {}
        }
        let metadata = fs::metadata(&marker).ok()?;
        let git_directory = if metadata.is_dir() {
            marker
        } else if metadata.is_file() {
            let pointer = read_git_metadata(&marker)?;
            let target = pointer.strip_prefix("gitdir:")?.trim();
            if target.is_empty() {
                return None;
            }
            let target = Path::new(target);
            if target.is_absolute() {
                target.to_path_buf()
            } else {
                directory.join(target)
            }
        } else {
            return None;
        };
        let head = read_git_metadata(&git_directory.join("HEAD"))?;
        if let Some(branch) = head.strip_prefix("ref: refs/heads/") {
            return (!branch.is_empty()
                && !branch
                    .chars()
                    .any(|value| value.is_control() || value.is_whitespace()))
            .then(|| branch.to_owned());
        }
        return (matches!(head.len(), 40 | 64)
            && head.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| format!("detached@{}", &head[..8]));
    }
    None
}

fn read_git_metadata(path: &Path) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > 4096 {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(4097)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 4096 {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    Some(text.trim().to_owned())
}

fn token_start(prefix: &str) -> usize {
    let (mut start, mut quoted, mut escaped) = (0, false, false);
    for (index, character) in prefix.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quoted {
            escaped = true;
        } else if character == '"' {
            quoted = !quoted;
        } else if character.is_whitespace() && !quoted {
            start = index + character.len_utf8();
        }
    }
    start
}

fn file_query(token: &str) -> (&str, &str) {
    if token.starts_with('@')
        && let Some((head, tail)) = token.rsplit_once(':')
        && !tail.is_empty()
        && tail
            .chars()
            .all(|value| value.is_ascii_digit() || matches!(value, 'L' | 'l' | '-'))
    {
        return (head, &token[head.len()..]);
    }
    (token, "")
}

fn entry(
    kind: CompletionKind,
    value: &str,
    label: &str,
    badge: &str,
    detail: &str,
    keywords: &str,
    submit_on_enter: bool,
) -> CatalogEntry {
    CatalogEntry {
        item: CompletionItem {
            kind,
            value: value.to_owned(),
            display_label: label.to_owned(),
            badge: badge.to_owned(),
            detail: detail.to_owned(),
            replace_start: 0,
            submit_on_enter,
        },
        keywords: keywords.to_owned(),
    }
}

fn command_entries() -> Vec<CatalogEntry> {
    let mut entries = COMMANDS
        .iter()
        .map(|value| {
            let prompt = matches!(
                *value,
                "/ask"
                    | "/prompt"
                    | "/sonnet"
                    | "/opus"
                    | "/argon"
                    | "/flash"
                    | "/dag-ulw"
                    | "/dag-run"
                    | "/dag"
                    | "/dag-plan"
                    | "/triad"
                    | "/triad-review"
            );
            let (detail, keywords) = command_description(value);
            entry(
                CompletionKind::Command,
                value,
                value,
                "CMD",
                detail,
                keywords,
                !prompt,
            )
        })
        .collect::<Vec<_>>();
    for command in ["/use", "/model"] {
        for (model, description) in [
            ("argon", "아르곤 제미나이 설계"),
            ("argon4", "아르곤4 제미나이"),
            ("sonnet", "소넷 클로드"),
            ("opus", "오퍼스 클로드"),
            ("flash", "플래시 제미나이"),
            ("gpt-6-astra", "아스트라 코덱스"),
            ("gpt-6-terra", "테라 코덱스"),
            ("gpt-6-luna", "루나 코덱스"),
            ("gpt-6-sol", "솔 코덱스"),
            ("gpt-6.1-terra", "테라 6.1 코덱스"),
            ("gpt-6.1-sol", "솔 6.1 코덱스"),
        ] {
            add_command(
                &mut entries,
                &format!("{command} {model}"),
                "MODEL",
                description,
            );
        }
    }
    for command in ["/skill", "/skills"] {
        for (preset, description) in [
            ("default", "기본 스킬"),
            ("core", "핵심 스킬 3개"),
            ("clear", "스킬 해제"),
            ("list", "스킬 목록"),
            ("off", "스킬 끄기"),
            ("none", "스킬 해제"),
            ("reset", "스킬 초기화"),
        ] {
            add_command(
                &mut entries,
                &format!("{command} {preset}"),
                "PRESET",
                description,
            );
        }
    }
    for value in ["on", "exit", "off"] {
        add_command(
            &mut entries,
            &format!("/dag-mode {value}"),
            "DAG",
            "DAG 모드 활성화 해제",
        );
    }
    for command in ["/prompts", "/promt", "/trace", "/prompt-trace"] {
        for value in ["argon", "claude", "codex", "all"] {
            add_command(
                &mut entries,
                &format!("{command} {value}"),
                "PROMPTS",
                "에이전트 프롬프트 조회",
            );
        }
    }
    for command in ["/permissions", "/approvals"] {
        for value in ["read-only", "auto-edit", "yolo"] {
            add_command(
                &mut entries,
                &format!("{command} {value}"),
                "PERM",
                "권한 승인 샌드박스",
            );
        }
    }
    for command in ["/effect", "/effort", "/reasoning", "/dag-effect"] {
        for value in ["reset", "show"] {
            add_command(
                &mut entries,
                &format!("{command} {value}"),
                "EFFECT",
                "에이전트 설정 초기화 보기",
            );
        }
        for style in ["neon-pulse", "cyber-matrix", "royal-prism", "minimal-mono"] {
            add_command(
                &mut entries,
                &format!("{command} style {style}"),
                "STYLE",
                "시각 효과 테마 스타일",
            );
        }
        for effort in ["low", "medium", "high", "xhigh", "max"] {
            add_command(
                &mut entries,
                &format!("{command} codex {effort}"),
                "EFFORT",
                "코덱스 추론 강도 에포트",
            );
        }
        for model in [
            "6 terra",
            "6 luna",
            "6 sol",
            "6 astra",
            "6-1 terra",
            "6-1 sol",
            "6.1 terra",
            "6.1 sol",
        ] {
            add_command(
                &mut entries,
                &format!("{command} codex {model} xhigh"),
                "MODEL",
                "코덱스 모델 추론 강도",
            );
        }
        for model in ["sonnet", "opus"] {
            add_command(
                &mut entries,
                &format!("{command} claude {model}"),
                "MODEL",
                if model == "opus" {
                    "오퍼스 클로드 모델"
                } else {
                    "소넷 클로드 모델"
                },
            );
        }
        for model in ["argon", "flash"] {
            add_command(
                &mut entries,
                &format!("{command} argon {model}"),
                "MODEL",
                "아르곤 플래시 모델",
            );
        }
    }
    for theme in ["codex-dark", "minimal-mono", "royal-prism", "cyber-matrix"] {
        add_command(
            &mut entries,
            &format!("/theme {theme}"),
            "THEME",
            "화면 테마 색상",
        );
    }
    entries
}

fn add_command(entries: &mut Vec<CatalogEntry>, value: &str, badge: &str, detail: &str) {
    entries.push(entry(
        CompletionKind::Command,
        value,
        value,
        badge,
        detail,
        detail,
        true,
    ));
}

fn command_description(command: &str) -> (&'static str, &'static str) {
    match command {
        "/skills" | "/skill" => ("등록된 스킬 보기/선택", "스킬 목록 카탈로그 core default"),
        "/dag-mode" | "/dag-exit" => ("DAG 모드 활성화/해제", "다그 모드 헤테라키 종료"),
        "/effect" | "/effort" | "/reasoning" | "/dag-effect" => {
            ("에이전트 모델·추론 강도·효과", "이펙트 에포트 추론 설정")
        }
        "/use" | "/model" | "/models" | "/lanes" => {
            ("모델 선택/라우팅 보기", "모델 아르곤 소넷 오퍼스 코덱스")
        }
        "/prompts" | "/promt" | "/trace" | "/prompt-trace" => {
            ("에이전트 프롬프트/결과 보기", "프롬프트 조회 트레이스")
        }
        "/back" | "/prev" => ("이전 화면으로 돌아가기", "이전 뒤로 복귀"),
        "/permissions" | "/approvals" => ("권한·승인 설정", "권한 승인 샌드박스"),
        "/compact" => ("로컬 기록·화면 정리", "컨텍스트 압축 요약 토큰 기록"),
        "/clear" => ("대화와 화면 초기화", "화면 지우기 초기화"),
        "/diff" | "/review" => ("변경 파일 비교/리뷰", "변경 코드 리뷰 감사 점검"),
        "/resume" => ("DAG 체크포인트 재개", "재개 이어하기 복구"),
        "/sonnet" => ("Claude Sonnet에게 질문", "소넷 클로드 질문"),
        "/opus" => ("Claude Opus에게 질문", "오퍼스 클로드 질문"),
        "/argon" => ("Argon에게 질문", "아르곤 제미나이 질문"),
        "/flash" => ("Flash에게 질문", "플래시 제미나이 질문"),
        "/dag-ulw" | "/dag-run" | "/dag" | "/dag-plan" | "/triad" => {
            ("DAG 계획/실행", "다그 그래프 목표 캠페인")
        }
        "/triad-review" => (
            "Argon·Claude·Codex 실제 3극 검토",
            "삼극 검토 합의 triad pipeline council",
        ),
        "/theme" => ("화면 테마 선택", "테마 색상 디자인"),
        "/help" => ("명령어와 단축키 도움말", "도움말 사용법"),
        "/quit" | "/exit" => ("TUI 종료", "종료 나가기 끝"),
        _ => ("명령어", "command"),
    }
}

fn contextual_command(prefix: &str) -> Option<&str> {
    let (command, _) = prefix.split_once(char::is_whitespace)?;
    matches!(
        command,
        "/use"
            | "/model"
            | "/skill"
            | "/skills"
            | "/dag-mode"
            | "/effect"
            | "/effort"
            | "/reasoning"
            | "/dag-effect"
            | "/prompts"
            | "/promt"
            | "/trace"
            | "/prompt-trace"
            | "/permissions"
            | "/approvals"
            | "/theme"
    )
    .then_some(command)
}

fn skill_key(value: &str) -> String {
    value
        .trim()
        .trim_start_matches(['$', '/'])
        .rsplit(':')
        .next()
        .unwrap_or_default()
        .to_lowercase()
}

fn core_keywords(name: &str) -> &'static str {
    match name {
        "codex-heterarchy" => "헤테라키 협업 삼극",
        "codegraph-operator" => "코드그래프 심볼 영향도",
        "codebase-refactor-guardian" => "가디언 리팩토링 코드 정리",
        _ => "",
    }
}

fn plugin_name(path: &Path) -> Option<String> {
    let components = path
        .components()
        .filter_map(|part| part.as_os_str().to_str())
        .collect::<Vec<_>>();
    let index = components.iter().rposition(|part| *part == "skills")?;
    let mut candidate = *components.get(index.checked_sub(1)?)?;
    if candidate
        .chars()
        .next()
        .is_some_and(|value| value.is_ascii_digit())
    {
        candidate = components.get(index.checked_sub(2)?)?;
    }
    (!candidate.is_empty()).then(|| candidate.to_owned())
}

fn discover_files(workspace: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    let walker = WalkDir::new(workspace)
        .follow_links(false)
        .sort_by_file_name()
        .max_depth(8)
        .into_iter();
    for entry in walker
        .filter_entry(include_entry)
        .filter_map(std::result::Result::ok)
    {
        if entry.file_type().is_file() || entry.file_type().is_dir() && entry.depth() > 0 {
            let relative = entry.path().strip_prefix(workspace)?;
            if let Some(path) = relative.to_str() {
                let path = if entry.file_type().is_dir() {
                    format!("{path}/")
                } else {
                    path.to_owned()
                };
                files.push(
                    if path.ends_with('.')
                        || path.chars().any(|value| {
                            value.is_whitespace()
                                || value.is_control()
                                || matches!(
                                    value,
                                    '"' | '\\'
                                        | '['
                                        | ']'
                                        | '('
                                        | ')'
                                        | '{'
                                        | '}'
                                        | '<'
                                        | '>'
                                        | ','
                                        | ';'
                                        | '`'
                                        | '\''
                                        | '@'
                                        | ':'
                                )
                        })
                    {
                        format!("@{}", serde_json::to_string(&path)?)
                    } else {
                        format!("@{path}")
                    },
                );
                if files.len() >= 8192 {
                    break;
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

fn include_entry(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    let name = entry.file_name().to_string_lossy();
    !matches!(
        name.as_ref(),
        ".git" | ".codegraph" | ".venv" | "target" | "node_modules"
    ) && !entry.path_is_symlink()
}

fn fuzzy_score(candidate: &str, needle: &str) -> Option<usize> {
    if candidate.starts_with(needle) {
        return Some(0);
    }
    let mut offset = 0usize;
    let mut score = 0usize;
    for character in needle.chars() {
        let found = candidate[offset..].find(character)?;
        score += found + 1;
        offset += found + character.len_utf8();
    }
    Some(score)
}

#[cfg(test)]
mod tests;
