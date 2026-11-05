use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::Result;
use crate::dag::{DagPlan, NodeRole, NodeState, SettlementReceipt};
use crate::routing::{SandboxMode, normalize_model};
use crate::tui::{
    DagLaneView, LaneState, TokenUsage, TranscriptEntry, TuiAction, TuiTheme, TuiUpdate,
};

use super::{HISTORY_LIMIT, HandlerState, PromptRecord};

#[derive(Clone)]
pub(super) struct PoleEffect {
    pub(super) model: String,
    pub(super) effort: String,
}

#[derive(Clone)]
pub(super) struct EffectConfig {
    pub(super) argon: PoleEffect,
    pub(super) claude: PoleEffect,
    pub(super) codex: PoleEffect,
    pub(super) style: TuiTheme,
}

impl Default for EffectConfig {
    fn default() -> Self {
        Self {
            argon: PoleEffect {
                model: "gemini-4-argon-high".to_owned(),
                effort: "high".to_owned(),
            },
            claude: PoleEffect {
                model: "claude-sonnet-5-5-medium".to_owned(),
                effort: "medium".to_owned(),
            },
            codex: PoleEffect {
                model: "gpt-6-astra".to_owned(),
                effort: "xhigh".to_owned(),
            },
            style: TuiTheme::CodexDark,
        }
    }
}

impl EffectConfig {
    pub(super) fn summary(&self) -> String {
        format!(
            "argon {}:{} · claude {}:{} · codex {}:{} · {}",
            self.argon.model,
            self.argon.effort,
            self.claude.model,
            self.claude.effort,
            self.codex.model,
            self.codex.effort,
            self.style.label()
        )
    }
}

pub(super) fn inline_skill_token(token: &str) -> bool {
    token.starts_with('$')
        || token.starts_with('/')
            && matches!(
                crate::tui::action_from_input(token),
                TuiAction::UnknownCommand(_)
            )
}

pub(super) fn direct_request(
    model: &str,
    effort: &str,
    prompt: &str,
    skill_context: &str,
    workspace: &Path,
) -> (&'static str, Value) {
    if model.starts_with("gpt-") {
        (
            "codex_targeted_synthesize",
            json!({
                "signature": "interactive_terminal_prompt",
                "contract_and_constraints": format!("{prompt}\n\n{skill_context}"),
                "context_types": "Workspace-grounded interactive request",
                "language": "rust",
                "include_unit_tests": false,
                "workspace_dir": workspace,
                "model": model,
                "reasoning_effort": effort
            }),
        )
    } else {
        (
            "claude_code_craft",
            json!({
                "task_spec": prompt,
                "target_code_or_context": "Interactive terminal request",
                "invariants_and_rules": skill_context,
                "language": "rust",
                "claude_model": model,
                "workspace_dir": workspace
            }),
        )
    }
}

pub(super) fn dag_lanes(plan: &DagPlan) -> Vec<DagLaneView> {
    plan.nodes
        .iter()
        .map(|node| {
            let scope_file = (node.role != NodeRole::IntegrationFanIn)
                .then(|| {
                    node.write_files
                        .first()
                        .or_else(|| node.read_files.first())
                        .and_then(|path| short_scope_file(path))
                })
                .flatten();
            let title = match scope_file {
                Some(file) if !node.title.contains(&file) => format!("{} · {file}", node.title),
                _ => node.title.clone(),
            };
            DagLaneView {
                id: node.id.clone(),
                layer: plan
                    .waves
                    .iter()
                    .position(|wave| wave.node_ids.contains(&node.id))
                    .unwrap_or(0),
                title,
                summary: node.task.clone(),
                role: node_role_label(&node.role, &node.model).to_owned(),
                model: node.model.clone(),
                effort: node.reasoning_effort.clone(),
                state: LaneState::Waiting,
                note: if node.dependencies.is_empty() {
                    "Start node".to_owned()
                } else {
                    format!("Waits on {}", node.dependencies.join(", "))
                },
                elapsed: None,
                dependencies: node.dependencies.clone(),
            }
        })
        .collect()
}

fn short_scope_file(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    if matches!(
        name,
        "tests.rs" | "helpers.rs" | "mod.rs" | "lib.rs" | "main.rs"
    ) && let Some(parent) = path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|parent| parent.to_str())
    {
        return Some(format!("{parent}/{name}"));
    }
    Some(name.to_owned())
}

pub(super) fn role_label(role: &NodeRole) -> &'static str {
    match role {
        NodeRole::TopologyPlanner => "architect",
        NodeRole::BoundaryMapper => "explore",
        NodeRole::InterfaceBuilder => "deep",
        NodeRole::ContractVerifier => "verify",
        NodeRole::SecurityAuditor => "review",
        NodeRole::IntegrationFanIn => "integrate",
    }
}

pub(super) fn node_role_label(role: &NodeRole, model: &str) -> &'static str {
    if matches!(role, NodeRole::InterfaceBuilder) && model.contains("opus") {
        "ultrabrain"
    } else {
        role_label(role)
    }
}

pub(super) fn apply_effect_argument(config: &mut EffectConfig, argument: &str) -> Result<()> {
    let normalized = argument.trim().to_ascii_lowercase();
    let mut next = config.clone();
    if matches!(normalized.as_str(), "reset" | "default") {
        *config = EffectConfig::default();
        return Ok(());
    }
    let tokens = normalized.split_whitespace().collect::<Vec<_>>();
    if tokens.first() == Some(&"style") {
        if tokens.len() != 2 {
            return Err("effect style requires exactly one theme".into());
        }
        next.style = parse_theme(tokens[1])?;
    } else if let Ok(theme) = parse_theme(&normalized) {
        next.style = theme;
    } else if tokens.first() == Some(&"all") {
        let effort = tokens
            .get(1)
            .copied()
            .filter(|effort| is_effort(effort))
            .filter(|_| tokens.len() == 2)
            .ok_or("effect all requires one supported effort")?;
        if effort != "medium" {
            return Err("effect all requires medium because Claude 5.5 supports medium only; configure Argon/Codex separately".into());
        }
        configure_gemini_effort(&mut next.argon, effort)?;
        next.claude.effort = effort.to_owned();
        next.codex.effort = effort.to_owned();
    } else {
        let (explicit_pole, values, initial_model) = match tokens.first().copied() {
            Some("argon") => (Some("argon"), &tokens[1..], None),
            Some("claude") => (Some("claude"), &tokens[1..], None),
            Some("sonnet" | "opus") => (Some("claude"), &tokens[1..], normalize_model(tokens[0])),
            Some("codex") => (Some("codex"), &tokens[1..], None),
            _ => (None, tokens.as_slice(), None),
        };
        let efforts = values
            .iter()
            .copied()
            .filter(|value| is_effort(value))
            .collect::<Vec<_>>();
        if efforts.len() > 1 {
            return Err("specify exactly one effort".into());
        }
        let effort = efforts.first().copied();
        let model_tokens = values
            .iter()
            .copied()
            .filter(|value| !is_effort(value))
            .collect::<Vec<_>>();
        let model = if model_tokens.is_empty() {
            initial_model
        } else {
            Some(
                resolve_effect_model(&model_tokens)
                    .ok_or_else(|| format!("unsupported model: {}", model_tokens.join(" ")))?,
            )
        };
        let pole = explicit_pole.unwrap_or_else(|| match model.as_deref() {
            Some(value) if value.starts_with("gemini-") => "argon",
            Some(value) if value.starts_with("claude-") => "claude",
            _ => "codex",
        });
        let profile = match pole {
            "argon" => &mut next.argon,
            "claude" => &mut next.claude,
            _ => &mut next.codex,
        };
        if let Some(model) = model {
            let valid = match pole {
                "argon" => model.starts_with("gemini-"),
                "claude" => model.starts_with("claude-"),
                _ => model.starts_with("gpt-"),
            };
            if !valid {
                return Err(format!("model {model} does not belong to {pole} lane").into());
            }
            if pole == "claude" && !model.ends_with("-medium") {
                return Err("Claude 5.5 lanes accept medium tier only".into());
            }
            profile.model = model;
        } else if effort.is_none() {
            return Err(format!("unknown effect argument: {argument}").into());
        }
        if let Some(effort) = effort {
            if pole == "claude" && effort != "medium" {
                return Err("Claude 5.5 lanes accept medium effort only".into());
            }
            if pole == "argon" {
                configure_gemini_effort(profile, effort)?;
            } else {
                profile.effort = effort.to_owned();
            }
        } else if pole == "argon" {
            profile.effort = profile
                .model
                .rsplit('-')
                .next()
                .unwrap_or("high")
                .to_owned();
        }
    }
    *config = next;
    Ok(())
}

pub(super) fn resolve_effect_model(tokens: &[&str]) -> Option<String> {
    let joined = tokens
        .iter()
        .filter(|token| !is_effort(token))
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    normalize_model(&joined)
}

pub(super) fn configure_gemini_effort(profile: &mut PoleEffect, effort: &str) -> Result<()> {
    if !matches!(effort, "low" | "medium" | "high") {
        return Err("Gemini model tiers are low, medium or high".into());
    }
    let (family, _) = profile
        .model
        .rsplit_once('-')
        .ok_or("Gemini model has no tier")?;
    let model = format!("{family}-{effort}");
    profile.model =
        normalize_model(&model).ok_or_else(|| format!("unsupported Gemini model tier: {model}"))?;
    profile.effort = effort.to_owned();
    Ok(())
}

pub(super) fn is_effort(value: &str) -> bool {
    matches!(value, "low" | "medium" | "high" | "xhigh" | "max" | "ultra")
}

pub(super) fn parse_theme(value: &str) -> Result<TuiTheme> {
    match value.trim().to_ascii_lowercase().as_str() {
        "codex-dark" | "tui-theme-boot" | "neon-pulse" | "default" => Ok(TuiTheme::CodexDark),
        "minimal-mono" | "mono" => Ok(TuiTheme::MinimalMono),
        "royal-prism" | "prism" => Ok(TuiTheme::RoyalPrism),
        "cyber-matrix" | "matrix" => Ok(TuiTheme::CyberMatrix),
        other => Err(format!("unknown theme: {other}").into()),
    }
}

pub(super) fn normalize_permission(value: &str) -> Result<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "read-only" | "readonly" | "read" => Ok("read-only".to_owned()),
        "auto-edit" | "auto" | "edit" | "workspace-write" => Ok("auto-edit".to_owned()),
        "yolo" | "yolo mode" | "full" | "danger-full-access" => Ok("yolo".to_owned()),
        other => Err(format!("unknown permission mode: {other}").into()),
    }
}

pub(super) fn permission_sandbox(value: &str) -> SandboxMode {
    match value {
        "read-only" => SandboxMode::ReadOnly,
        "yolo" => SandboxMode::DangerFullAccess,
        _ => SandboxMode::WorkspaceWrite,
    }
}

pub(super) fn ensure_plan_permission(mode: &str, plan: &DagPlan, execute: bool) -> Result<()> {
    if execute && mode == "read-only" && plan.nodes.iter().any(|node| !node.write_files.is_empty())
    {
        return Err("read-only permission blocks DAG lanes with declared writes".into());
    }
    Ok(())
}

pub(super) fn apply_effects_to_plan(plan: &mut DagPlan, effects: &EffectConfig) {
    let defaults = EffectConfig::default();
    let claude_customized = effects.claude.model != defaults.claude.model
        || effects.claude.effort != defaults.claude.effort;
    let codex_customized = effects.codex.model != defaults.codex.model
        || effects.codex.effort != defaults.codex.effort;
    for node in &mut plan.nodes {
        match node.role {
            NodeRole::TopologyPlanner | NodeRole::IntegrationFanIn => {
                node.model.clone_from(&effects.argon.model);
                node.reasoning_effort.clone_from(&effects.argon.effort);
            }
            NodeRole::InterfaceBuilder => {
                if claude_customized || !node.model.starts_with("claude-") {
                    node.model.clone_from(&effects.claude.model);
                    node.reasoning_effort.clone_from(&effects.claude.effort);
                }
            }
            NodeRole::BoundaryMapper | NodeRole::ContractVerifier | NodeRole::SecurityAuditor => {
                if codex_customized || node.role == NodeRole::SecurityAuditor {
                    node.model.clone_from(&effects.codex.model);
                    node.reasoning_effort.clone_from(&effects.codex.effort);
                }
            }
        }
    }
}

pub(super) fn dag_request_nodes(plan: &DagPlan, skill_context: &str) -> Vec<Value> {
    plan.nodes
        .iter()
        .map(|node| {
            let role = match node.role {
                NodeRole::TopologyPlanner => "topology_planner",
                NodeRole::BoundaryMapper => "interface_contractor",
                NodeRole::InterfaceBuilder => "disjoint_builder",
                NodeRole::ContractVerifier => "reproduction_prober",
                NodeRole::SecurityAuditor => "adversarial_auditor",
                NodeRole::IntegrationFanIn => "verification_fan_in",
            };
            json!({
                "node_id": node.id,
                "role": role,
                "node_kind": node.node_kind,
                "task": if skill_context.is_empty() { node.task.clone() } else {
                    format!("{}\n\n{}", node.task, skill_context)
                },
                "deliverable": node.title,
                "read_files": node.read_files,
                "write_files": node.write_files,
                "symbols": node.symbols,
                "depends_on": node.dependencies,
                "verify_cmd": node.verify_command.clone().unwrap_or_default(),
                "stop_when": "typed deliverable and verification evidence are recorded",
                "preferred_model": node.model,
                "reasoning_effort": node.reasoning_effort,
                "reproduction_expected_exit_nonzero": node.reproduction_expected_exit_nonzero
            })
        })
        .collect()
}

pub(super) fn prompt_inspection_lines(
    request: &Value,
    result: Option<&Value>,
    selector: Option<&str>,
) -> Vec<String> {
    let filter = selector.unwrap_or_default().trim().to_ascii_lowercase();
    let mut lines = vec![
        format!(
            "Goal: {}",
            request["campaign_goal"].as_str().unwrap_or_default()
        ),
        "Recorded typed node instructions and actual returned artifacts. Transport grounding/repair prompts are not reproduced here.".to_owned(),
    ];
    let nodes = request["nodes"].as_array().map_or(&[][..], Vec::as_slice);
    let mut matches = 0;
    for (index, node) in nodes.iter().enumerate() {
        let id = node["node_id"].as_str().unwrap_or_default();
        let model = node["preferred_model"].as_str().unwrap_or_default();
        let role = node["role"].as_str().unwrap_or_default();
        let pole = if model.starts_with("gemini-") {
            "argon"
        } else if model.starts_with("claude-") {
            "claude"
        } else {
            "codex"
        };
        if !filter.is_empty()
            && filter != "all"
            && filter != id.to_ascii_lowercase()
            && !id.to_ascii_lowercase().starts_with(&format!("{filter}_"))
            && filter != pole
            && filter != role
            && filter != (index + 1).to_string()
            && filter != format!("w{index}")
        {
            continue;
        }
        matches += 1;
        lines.push(format!(
            "[{id}] {pole} · {model} · {}",
            node["reasoning_effort"].as_str().unwrap_or_default()
        ));
        lines.push(format!(
            "1. TASK & REGISTERED SKILLS (actual typed task; bounded excerpt)\n{}",
            truncate_utf8(node["task"].as_str().unwrap_or_default(), 6_000)
        ));
        lines.push(format!(
            "2. DELIVERABLE\n{}",
            node["deliverable"].as_str().unwrap_or_default()
        ));
        lines.push(format!(
            "3. UPSTREAM / SCOPE / VERIFY\nparents={}\nread={}\nwrite={}\nverify={}",
            node["depends_on"], node["read_files"], node["write_files"], node["verify_cmd"]
        ));
        let receipt = result
            .and_then(|r| r["node_receipts"].as_array())
            .and_then(|items| items.iter().find(|r| r["node_id"] == id));
        let output = result.and_then(|r| r["outputs"].get(id));
        lines.push(format!(
            "4. ACTUAL AGENT OUTPUT\n{}",
            output.map_or_else(
                || "No returned provider artifact recorded for this node.".to_owned(),
                |value| pretty_json_limited(value, 3_000)
            )
        ));
        lines.push(format!(
            "5. SETTLEMENT & GATE EVIDENCE\n{}",
            receipt.map_or_else(
                || "Planned / unverified; no settlement receipt recorded.".to_owned(),
                |value| pretty_json_limited(value, 3_000)
            )
        ));
    }
    if matches == 0 {
        lines.push(format!(
            "No node matches '{filter}'. Use /prompts <node_id|argon|claude|codex>."
        ));
    }
    lines
}

pub(super) fn latest_dag_checkpoint(value: &Value, workspace: &Path) -> Option<String> {
    value["jobs"].as_array()?.iter().find_map(|job| {
        if Path::new(job["workspace_dir"].as_str()?) != workspace {
            return None;
        }
        match job["task_type"].as_str()? {
            "dag_ulw" if job["result"]["plan_only"] != true => {
                job["job_id"].as_str().map(str::to_owned)
            }
            "dag_checkpoint"
                if job["status"] == "completed" && job["metadata"]["checkpoint_kind"] == "plan" =>
            {
                let campaign = job["metadata"]["campaign_id"].as_str()?;
                (job["job_id"] == format!("ckp-{campaign}-plan")).then(|| campaign.to_owned())
            }
            _ => None,
        }
    })
}

pub(super) fn inline_resume_id(input: &str) -> Result<Option<&str>> {
    let mut parts = input.split_whitespace();
    let mut command = parts.next();
    if command == Some("--live") {
        command = parts.next();
    }
    if command != Some("--resume") {
        return Ok(None);
    }
    let id = parts.next().ok_or("--resume requires a campaign ID")?;
    if parts.next().is_some() {
        return Err("resume uses the stored plan; a replacement goal is not accepted".into());
    }
    Ok(Some(id))
}

pub(super) fn provider_stage(output: &Value) -> String {
    let status = value_at(output, &["result", "receipt", "status"])
        .or_else(|| value_at(output, &["receipt", "status"]))
        .and_then(Value::as_str)
        .unwrap_or("reported_done");
    let outcome = value_at(output, &["result", "receipt", "outcome_state"])
        .or_else(|| value_at(output, &["receipt", "outcome_state"]))
        .and_then(Value::as_str)
        .unwrap_or("evidence pending");
    format!("{status} · {outcome}")
}

pub(super) fn inspect_node_source(workspace: &Path, node: &crate::dag::DagNode) -> Option<String> {
    let rel_path = node
        .write_files
        .first()
        .or_else(|| node.read_files.first())?;
    let full_path = workspace.join(rel_path);
    let content = std::fs::read_to_string(&full_path).ok()?;
    let lines = content.lines().collect::<Vec<_>>();
    let line_count = lines.len();
    let mut symbols = Vec::new();
    let mut first_decl: Option<(usize, String)> = None;
    for (idx, raw) in lines.iter().enumerate() {
        let trimmed = raw.trim();
        for keyword in [
            "pub async fn ",
            "async fn ",
            "pub fn ",
            "fn ",
            "pub struct ",
            "struct ",
            "pub enum ",
            "enum ",
            "pub trait ",
            "trait ",
            "impl ",
            "CREATE TABLE ",
            "SELECT ",
        ] {
            if let Some(rest) = trimmed.strip_prefix(keyword) {
                let ident = rest
                    .split(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != ':')
                    .find(|token| !token.is_empty())
                    .unwrap_or("item");
                let label = format!("{ident}(L{})", idx + 1);
                if symbols.len() < 4 && !symbols.contains(&label) {
                    symbols.push(label);
                }
                if first_decl.is_none() {
                    first_decl = Some((idx + 1, truncate_utf8(trimmed, 72).to_owned()));
                }
                break;
            }
        }
    }
    let sym_text = if symbols.is_empty() {
        "module_root".to_owned()
    } else {
        symbols.join(", ")
    };
    let preview = first_decl.map_or_else(
        || {
            lines
                .iter()
                .position(|line| !line.trim().is_empty())
                .map_or_else(
                    || "[L1] (empty)".to_owned(),
                    |idx| format!("[L{}] {}", idx + 1, truncate_utf8(lines[idx].trim(), 72)),
                )
        },
        |(line_no, decl)| format!("[L{line_no}] {decl}"),
    );
    Some(format!(
        "ast_scan(\"{}\", lines={line_count}, symbols=\"{sym_text}\") — {preview}",
        rel_path.display()
    ))
}

pub(super) fn provider_output_preview(
    node: Option<&crate::dag::DagNode>,
    output: &Value,
) -> Option<String> {
    let structured = value_at(output, &["result", "structured_output"])
        .or_else(|| value_at(output, &["structured_output"]))
        .or(Some(output))?;
    let tag = match node.map(|n| (&n.role, n.model.as_str())) {
        Some((NodeRole::InterfaceBuilder, model)) if model.contains("opus") => "opus_arch",
        Some((NodeRole::InterfaceBuilder, _)) => "code_craft",
        Some((NodeRole::SecurityAuditor | NodeRole::ContractVerifier, _)) => "red_team_audit",
        Some((
            NodeRole::TopologyPlanner | NodeRole::BoundaryMapper | NodeRole::IntegrationFanIn,
            _,
        )) => "synthesis",
        None => "agent_work",
    };
    if let Some(edits) = structured.get("workspace_edits").and_then(Value::as_array)
        && let Some(first) = edits.first()
    {
        let path = first
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("workspace");
        let content = first
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let line_count = content.lines().count();
        let snippet = content
            .lines()
            .enumerate()
            .find(|(_, line)| {
                let t = line.trim();
                !t.is_empty() && !t.starts_with("//") && !t.starts_with("use ")
            })
            .map_or_else(
                || "staged patch ready".to_owned(),
                |(idx, line)| format!("[L{}] {}", idx + 1, truncate_utf8(line.trim(), 68)),
            );
        return Some(format!(
            "{tag}(\"{path}\", {line_count}L, unverified preview) — {snippet}"
        ));
    }
    let text = [
        "summary",
        "executive_critique",
        "implementation_code",
        "crafted_code",
    ]
    .into_iter()
    .find_map(|key| {
        structured
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    })?;
    let first_line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(text);
    let scope = node
        .and_then(|n| n.write_files.first().or_else(|| n.read_files.first()))
        .map_or_else(|| "workspace".to_owned(), |p| p.display().to_string());
    Some(format!(
        "{tag}(\"{scope}\", unverified preview) — {}",
        truncate_utf8(first_line.trim(), 76)
    ))
}

pub(super) fn settlement_view(receipt: &SettlementReceipt) -> (LaneState, String) {
    match receipt.state {
        NodeState::Settled => (
            LaneState::Accepted,
            format!(
                "SETTLED · verified by {} · {}",
                receipt.gate.command,
                receipt
                    .gate
                    .stdout_digest
                    .as_deref()
                    .unwrap_or("digest missing")
            ),
        ),
        NodeState::Stopped => (
            LaneState::Stopped,
            "STOPPED · reported done; verification incomplete".to_owned(),
        ),
        NodeState::Planned => (LaneState::Waiting, "Plan · not run".to_owned()),
        NodeState::Running => (LaneState::Running, "Code · running".to_owned()),
        NodeState::RejectedOutOfScopeWrite => (
            LaneState::Rejected,
            format!(
                "rejected · out-of-scope writes: {}",
                receipt
                    .out_of_scope_writes
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        NodeState::RejectedVerification => (
            LaneState::Rejected,
            format!(
                "verification rejected · {} · exit {:?}",
                receipt.gate.command, receipt.gate.exit_code
            ),
        ),
        NodeState::BlockedUpstream => (
            LaneState::Rejected,
            "blocked by unverified upstream lane".to_owned(),
        ),
        NodeState::RejectedDirtyRead => (
            LaneState::Rejected,
            format!(
                "rejected · dirty reads: {}",
                receipt
                    .dirty_read_files
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ),
        NodeState::RejectedReproductionNotObserved => (
            LaneState::Rejected,
            "rejected · expected reproduction failure was not observed".to_owned(),
        ),
        NodeState::FailedExecution => (
            LaneState::Rejected,
            format!(
                "execution failed · {}",
                receipt.error.as_deref().unwrap_or("unspecified error")
            ),
        ),
    }
}

pub(super) fn push_history(history: &mut VecDeque<PromptRecord>, request: String, response: Value) {
    if history.len() == HISTORY_LIMIT {
        history.pop_front();
    }
    history.push_back(PromptRecord { request, response });
}

pub(super) fn update_usage_state(state: &mut HandlerState, value: &Value) -> TokenUsage {
    let request_budget = extract_request_budget(value);
    let receipts = collect_receipts(value);
    if receipts.is_empty() {
        state.usage_unknown_seen = true;
    }
    for (request_id, receipt) in receipts {
        if state.seen_usage_requests.insert(request_id) {
            let input = receipt
                .get("tokens_in")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let output = receipt
                .get("tokens_out")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            if input == 0 && output == 0 {
                state.usage_unknown_seen = true;
            } else {
                state.usage_known_seen = true;
                state.tokens_in = state.tokens_in.saturating_add(input);
                state.tokens_out = state.tokens_out.saturating_add(output);
            }
        }
    }
    state.context_window = None;
    state.current_context_tokens = None;
    let context = extract_context_usage(value);
    if let Some((window, current, _)) = &context {
        state.context_window = Some(*window);
        state.current_context_tokens = Some(*current);
    }
    TokenUsage {
        input: state.usage_known_seen.then_some(state.tokens_in),
        output: state.usage_known_seen.then_some(state.tokens_out),
        context_limit: state.context_window,
        current_context: state.current_context_tokens,
        usage_complete: state.usage_known_seen && !state.usage_unknown_seen,
        request_input_bytes: request_budget.map(|(input, _)| input),
        request_limit_bytes: request_budget.map(|(_, limit)| limit),
        context_origin_model: context.and_then(|(_, _, origin)| origin),
    }
}

pub(super) fn extract_request_budget(value: &Value) -> Option<(u64, u64)> {
    fn visit(value: &Value, budgets: &mut BTreeSet<(u64, u64)>) {
        match value {
            Value::Object(object) => {
                if let Some(budget) = object.get("budget")
                    && let (Some(input), Some(limit)) = (
                        budget.get("actual_input_bytes").and_then(Value::as_u64),
                        budget.get("max_input_bytes").and_then(Value::as_u64),
                    )
                    && limit > 0
                {
                    budgets.insert((input, limit));
                }
                for key in [
                    "result",
                    "roles",
                    "outputs",
                    "node_outputs",
                    "job",
                    "jobs",
                    "response",
                    "execution",
                    "argon_review",
                    "sonnet_craft",
                    "codex_audit",
                ] {
                    if let Some(child) = object.get(key) {
                        visit(child, budgets);
                    }
                }
            }
            Value::Array(values) => {
                for value in values {
                    visit(value, budgets);
                }
            }
            _ => {}
        }
    }
    let mut budgets = BTreeSet::new();
    visit(value, &mut budgets);
    (budgets.len() == 1)
        .then(|| budgets.into_iter().next())
        .flatten()
}

#[cfg(test)]
pub(super) fn extract_usage(value: &Value) -> (u64, u64) {
    let receipts = collect_receipts(value);
    receipts.values().fold((0u64, 0u64), |total, receipt| {
        let input = receipt
            .get("tokens_in")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let output = receipt
            .get("tokens_out")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        (
            total.0.saturating_add(input),
            total.1.saturating_add(output),
        )
    })
}

pub(super) fn collect_receipts(value: &Value) -> BTreeMap<String, &serde_json::Map<String, Value>> {
    fn visit<'a>(
        value: &'a Value,
        receipts: &mut BTreeMap<String, &'a serde_json::Map<String, Value>>,
    ) {
        match value {
            Value::Object(object) => {
                if let Some(receipt) = object.get("receipt").and_then(Value::as_object)
                    && let Some(request_id) = receipt.get("request_id").and_then(Value::as_str)
                {
                    receipts
                        .entry(receipt_identity(request_id, receipt))
                        .or_insert(receipt);
                }
                for key in [
                    "result",
                    "roles",
                    "outputs",
                    "node_outputs",
                    "job",
                    "jobs",
                    "response",
                    "execution",
                    "argon_review",
                    "sonnet_craft",
                    "codex_audit",
                ] {
                    if let Some(child) = object.get(key) {
                        visit(child, receipts);
                    }
                }
            }
            Value::Array(items) => {
                for child in items {
                    visit(child, receipts);
                }
            }
            _ => {}
        }
    }
    let mut receipts = BTreeMap::new();
    visit(value, &mut receipts);
    receipts
}

fn receipt_identity(request_id: &str, receipt: &serde_json::Map<String, Value>) -> String {
    let attempt = receipt
        .get("attempt_count")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let session = receipt
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("-");
    format!("{request_id}:attempt-{attempt}:session-{session}")
}

pub(super) fn extract_context_usage(value: &Value) -> Option<(u64, u64, Option<String>)> {
    let receipts = collect_receipts(value);
    let mut found = BTreeMap::<String, (u64, u64, Option<String>)>::new();
    for (provenance, receipt) in receipts {
        if let (Some(window), Some(current)) = (
            receipt.get("context_window").and_then(Value::as_u64),
            receipt
                .get("current_context_tokens")
                .and_then(Value::as_u64),
        ) && window > 0
        {
            let origin = receipt
                .get("actual_model")
                .and_then(Value::as_str)
                .map(str::to_owned);
            found.insert(provenance, (window, current, origin));
        }
    }
    if found.len() == 1 {
        found.values().next().cloned()
    } else {
        None
    }
}

pub(super) fn response_text(value: &Value) -> String {
    for path in [
        &["result", "structured_output"][..],
        &["structured_output"][..],
    ] {
        if let Some(output) = value_at(value, path) {
            for key in [
                "implementation_code",
                "crafted_code",
                "executive_critique",
                "summary",
            ] {
                if let Some(text) = output
                    .get(key)
                    .and_then(Value::as_str)
                    .filter(|text| !text.trim().is_empty())
                {
                    return truncate_utf8(text, 8_000).to_owned();
                }
            }
        }
    }
    for path in [
        &["result", "output"][..],
        &["result", "raw_output"][..],
        &["output"][..],
        &["summary"][..],
    ] {
        if let Some(text) = value_at(value, path).and_then(Value::as_str) {
            return truncate_utf8(text, 8_000).to_owned();
        }
    }
    pretty_json_limited(value, 8_000)
}

pub(super) fn value_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |current, key| current.get(key))
}

pub(super) fn pretty_json_limited(value: &Value, max_bytes: usize) -> String {
    serde_json::to_string_pretty(value).map_or_else(
        |_| "Response serialization failed".to_owned(),
        |body| truncate_utf8(&body, max_bytes).to_owned(),
    )
}

pub(super) fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

pub(super) async fn send_transcript(
    updates: &mpsc::Sender<TuiUpdate>,
    message: impl Into<String>,
) -> Result<()> {
    updates
        .send(TuiUpdate::Transcript(TranscriptEntry::root(message)))
        .await?;
    Ok(())
}

pub(super) async fn show_static_view(
    updates: &mpsc::Sender<TuiUpdate>,
    title: &str,
    body: Vec<String>,
) -> Result<()> {
    updates
        .send(TuiUpdate::View {
            title: title.to_owned(),
            body,
        })
        .await?;
    Ok(())
}

pub(super) fn models_summary(
    health: &Value,
    active_model: &str,
    active_effort: &str,
    effect_summary: &str,
) -> Vec<String> {
    let agy = &health["auth_boundaries"]["antigravity_cli"];
    let agy_status = if agy["readonly_admission_verified"].as_bool() == Some(true)
        && agy["catalog_ready"].as_bool() == Some(true)
    {
        "policy verified"
    } else if agy["ready"].as_bool() == Some(true) {
        "binary available"
    } else {
        "unavailable"
    };
    let codex_ready = health["codex_cli_routable"].as_bool().unwrap_or(false);
    vec![
        format!("Models & Routing Page [2:Models] · Active: {active_model}:{active_effort}"),
        format!("Effect Matrix: {effect_summary}"),
        format!(
            "CLI Readiness: Antigravity={agy_status} · Codex={}",
            if codex_ready {
                "● routable"
            } else {
                "○ offline"
            }
        ),
        "────────────────────────────────────────────────────────────────────────────────────────"
            .to_owned(),
    ]
}

pub(super) fn status_summary(health: &Value, jobs: &Value, inbox: &Value) -> Vec<String> {
    let status = health["status"].as_str().unwrap_or("unknown");
    let job_count = jobs["jobs"].as_array().map_or(0, Vec::len);
    let handoff_count = inbox["handoffs"].as_array().map_or(0, Vec::len);
    let mut lines = vec![
        format!(
            "Operational Status Page [6:Status] · Backend: {status} · Jobs: {job_count} · Handoffs: {handoff_count}"
        ),
        "────────────────────────────────────────────────────────────────────────────────────────"
            .to_owned(),
    ];
    lines.extend(runtime_identity_summary(&health["runtime_identity"]));
    lines
}

pub(super) fn inbox_summary(inbox: &Value) -> Vec<String> {
    let handoff_count = inbox["handoffs"].as_array().map_or(0, Vec::len);
    vec![
        format!("Handoff Inbox · Recorded items: {handoff_count}"),
        "────────────────────────────────────────────────────────────────────────────────────────"
            .to_owned(),
    ]
}

pub(super) fn workboard_summary(board: &Value) -> Vec<String> {
    let mut lines = vec![format!(
        "Organization Workboard · Workspace: {} · Records: {}",
        board["workspace_dir"].as_str().unwrap_or("unknown"),
        board["count"].as_u64().unwrap_or(0)
    )];
    lines.push(
        "────────────────────────────────────────────────────────────────────────────────────────"
            .to_owned(),
    );
    lines.extend(runtime_identity_summary(&board["runtime_identity"]));
    for item in board["items"].as_array().into_iter().flatten() {
        let detail = &item["detail"];
        let kind = detail["kind"].as_str().unwrap_or("record");
        let mut line = format!(
            "{} · {} · {} · {}",
            item["job_id"].as_str().unwrap_or("unknown"),
            item["task_type"].as_str().unwrap_or("unknown"),
            item["status"].as_str().unwrap_or("unknown"),
            kind
        );
        if kind == "node" {
            line.push_str(&format!(
                " · node={} · handoff={}",
                detail["node_id"].as_str().unwrap_or("unknown"),
                detail["handoff_status"].as_str().unwrap_or("unavailable")
            ));
            if let Some(summary) = detail["handoff"]["summary"].as_str()
                && !summary.is_empty()
            {
                line.push_str(&format!(" · {}", truncate_utf8(summary, 240)));
            }
        } else if kind == "review_council" {
            line.push_str(&format!(
                " · council={} · approved={}",
                detail["consensus_status"].as_str().unwrap_or("unknown"),
                detail["review_approved"].as_bool().unwrap_or(false)
            ));
        }
        lines.push(line);
    }
    if board["items"].as_array().is_none_or(Vec::is_empty) {
        lines.push(
            "No persisted campaigns, handoffs, or Review Councils in this workspace.".to_owned(),
        );
    }
    lines
}

fn runtime_identity_summary(identity: &Value) -> Vec<String> {
    if !identity.is_object() {
        return Vec::new();
    }
    let text = |value: &Value| {
        truncate_utf8(value.as_str().unwrap_or("unknown"), 128)
            .chars()
            .flat_map(char::escape_debug)
            .collect::<String>()
    };
    let source = &identity["source"];
    let dirty = source["dirty"]
        .as_bool()
        .map_or("unknown", |dirty| if dirty { "dirty" } else { "clean" });
    vec![
        format!(
            "Source identity · {} · {dirty} · HEAD={} · scope={}",
            text(&source["state"]),
            text(&source["head"]),
            text(&source["scope"])
        ),
        format!(
            "Installed CLI disk · {} · {}",
            text(&identity["installed_cli"]["state"]),
            text(&identity["installed_cli"]["sha256"])
        ),
        format!(
            "Current process · package={} · loaded image unverified",
            text(&identity["current_process"]["package_version"])
        ),
        "Source-to-binary linkage unverified; CI evidence not supplied by this observation."
            .to_owned(),
    ]
}

pub(super) fn votes_summary(votes: &Value) -> Vec<String> {
    let source_status = votes["source_status"]["status"]
        .as_str()
        .unwrap_or("unknown");
    let mut lines = vec![format!(
        "Review Council {} · source={} · historical_approved={} · current_approved={}",
        votes["collaboration_id"].as_str().unwrap_or("unknown"),
        source_status,
        votes["historical_review_approved"]
            .as_bool()
            .unwrap_or(false),
        votes["current_review_approved"].as_bool().unwrap_or(false)
    )];
    lines.push("Role · Actual model · Verdict · Validity".to_owned());
    lines.push(
        "────────────────────────────────────────────────────────────────────────────────────────"
            .to_owned(),
    );
    for ballot in votes["ballots"].as_array().into_iter().flatten() {
        lines.push(format!(
            "{} · {} · {} · {} ({})",
            ballot["role"].as_str().unwrap_or("unknown"),
            ballot["actual_model"].as_str().unwrap_or("unknown"),
            ballot["verdict"].as_str().unwrap_or("missing"),
            if ballot["valid"].as_bool().unwrap_or(false) {
                "valid"
            } else {
                "invalid"
            },
            ballot["reason"].as_str().unwrap_or("unknown")
        ));
    }
    lines
}

pub(super) fn recovery_summary(report: &Value) -> Vec<String> {
    let text = |value: &Value| {
        truncate_utf8(value.as_str().unwrap_or("unknown"), 240)
            .chars()
            .flat_map(char::escape_debug)
            .collect::<String>()
    };
    let mut lines = vec![
        format!(
            "Recovery {} · {} ({})",
            text(&report["job_id"]),
            text(&report["observation"]),
            text(&report["reason"])
        ),
        "Read-only observations; no cleanup, retry, or resume authorization.".to_owned(),
    ];
    let job = &report["job"];
    if job.is_object() {
        lines.push(format!(
            "Recorded job · {} · {} · model={}",
            text(&job["status"]),
            text(&job["outcome_state"]),
            text(&job["model"])
        ));
    }
    let worktrees = &report["worktrees"];
    if worktrees.is_object() {
        lines.push(format!(
            "Worktrees · {} ({})",
            text(&worktrees["observation"]),
            text(&worktrees["reason"])
        ));
        lines.push(format!("State path · {}", text(&worktrees["state_path"])));
        for node in worktrees["nodes"].as_array().into_iter().flatten() {
            let flag = |key: &str| {
                node[key]
                    .as_bool()
                    .map_or("unknown", |value| if value { "true" } else { "false" })
            };
            lines.push(format!(
                "{} · observed={} · path={} · Git={}",
                text(&node["node_id"]),
                text(&node["observation"]),
                text(&node["filesystem"]),
                text(&node["git_registration"])
            ));
            lines.push(format!(
                "  Recorded only · verified={} · removed={} · {}",
                flag("recorded_verified"),
                flag("recorded_removed"),
                text(&node["path"])
            ));
        }
    }
    lines
}

pub(super) fn help_lines() -> Vec<String> {
    vec![
        "Workbench Pages (press 1..7 inside any view when composer is empty):".to_owned(),
        "[1:Skills] [2:Models] [3:Board] [4:Prompts] [5:Diff] [6:Status] [7:Help]".to_owned(),
        "────────────────────────────────────────────────────────────────────────────────────────".to_owned(),
        "/skills [core|default|clear] · interactive Skills Configuration Page (● active / ○ deactivate)".to_owned(),
        "/skill <name...> · toggle registered skills; $skill or /registered-skill prefixes activate inline".to_owned(),
        "/dag-run [--worktree] <goal> · compile and execute a dynamic graph".to_owned(),
        "/dag-plan <goal> · compile only; nothing runs".to_owned(),
        "/prompts [/promt|/prompt-trace] [node_id|argon|claude|codex] · inspect actual node task/context/output/gates".to_owned(),
        "/models · provider routing health".to_owned(),
        "/use <model alias> [effort] · select the active model and its validated effort".to_owned(),
        "/effect [/dag-effect] <argon|claude|codex|all> <model/effort>; style <theme>; matrix; reset".to_owned(),
        "@path[:Lstart-Lend] · load actual workspace file context; unsafe/missing references are rejected".to_owned(),
        "/permissions [/approvals] · read-only, auto-edit, or yolo sandbox".to_owned(),
        "/resume [dag_id] · resume a matching durable DAG checkpoint".to_owned(),
        "/triad-review <goal> · actual Argon + Claude + Codex review pipeline".to_owned(),
        "/board [limit] · persisted campaign, node handoff and Review Council workboard (no model call)".to_owned(),
        "/council [--file relative/path ... --] <proposal> · run the Review Council; quoted paths are CLI-only".to_owned(),
        "/votes <collaboration_id> · actual role ballots, receipts and scoped source freshness".to_owned(),
        "/recovery <job_id> · read-only job and worktree observations; no cleanup or retry".to_owned(),
        "/research [--file path|--document path|--url HTTPS] [--plan-only] -- question · source-linked read-only research".to_owned(),
        "/research-status <request_id> · observe the saved research report; no source fetch or provider call".to_owned(),
        "/compact · compact local view/prompt history without rewriting receipt usage".to_owned(),
        "/theme [name] · inspect or change the active rendered palette".to_owned(),
        "/diff · SHA-256 changes since session baseline, including non-Git/untracked files".to_owned(),
        "/board [/plan|/todo|/omh] · actual DAG instructions, scope and workspace source/evidence board".to_owned(),
        "/review [focus] · real Codex working-tree review".to_owned(),
        "/back · return from a detail view".to_owned(),
        "/quit · cancel active terminal actions and exit".to_owned(),
    ]
}
/// Consume only a leading mode flag; a flag-like word inside a goal is content.
pub(super) fn worktree_goal(input: &str) -> (String, bool) {
    let input = input.trim();
    if input == "--worktree" {
        (String::new(), true)
    } else if let Some(rest) = input.strip_prefix("--worktree")
        && rest.starts_with(char::is_whitespace)
    {
        (rest.trim().to_owned(), true)
    } else {
        (input.to_owned(), false)
    }
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn identity_views_show_uncertainty_and_keep_empty_board_message() {
        let identity = json!({
            "source":{"state":"observed","dirty":true,"head":"abc123","scope":"git_listed_rust_sql_cargo_toolchain"},
            "installed_cli":{"state":"observed","sha256":"sha256:fixture"},
            "current_process":{"package_version":"0.3.0\u{1b}[31m"},
            "capabilities":{"backend":"in_process","mcp_server":false}
        });
        let board = workboard_summary(&json!({"items":[],"runtime_identity":identity}));
        assert!(board.iter().any(|line| line.contains("dirty")));
        assert!(
            board
                .iter()
                .any(|line| line.contains("Installed CLI disk · observed · sha256:fixture"))
        );
        assert!(
            board
                .iter()
                .any(|line| line.contains("No persisted campaigns"))
        );
        assert!(
            board
                .iter()
                .any(|line| line.contains("Source-to-binary linkage unverified"))
        );
        assert!(board.iter().all(|line| !line.contains('\u{1b}')));
        let status = status_summary(
            &json!({"runtime_identity":identity}),
            &json!({}),
            &json!({}),
        );
        assert!(
            status
                .iter()
                .any(|line| line.contains("CI evidence not supplied"))
        );
    }
}
