//! Prompt grounding, skill synchronization, route construction, and schema validation.

use super::contracts::prompt_prefix;
use super::optional;
use crate::context::{SkillRegistry, SourceContextBuilder};
use crate::routing::{Engine, Provider, RouteRequest};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static BOUNDARY_SCHEMAS: LazyLock<Option<Value>> =
    LazyLock::new(|| serde_json::from_str(include_str!("../../model-output-schemas.json")).ok());

static ACTIVE_SKILLS_MARKER: LazyLock<Option<regex::Regex>> = LazyLock::new(|| {
    regex::Regex::new(r"(?m)^=== ACTIVE REGISTERED SKILLS \(([^\r\n)]*)\) ===\r?$").ok()
});

pub(super) fn structured_prompt(prompt: &str, schema: &Value) -> crate::Result<String> {
    Ok(format!(
        "{prompt}\n\nSERVER-OWNED RESPONSE CONTRACT\nReturn exactly one raw JSON object conforming to the following JSON Schema. No Markdown fences, extra fields, receipts, evidence_scope, tool-call claims or prose outside the object. Runtime owns receipt/evidence generation. Include every required field; use empty values only when allowed and truthful.\n{}",
        serde_json::to_string(schema)?
    ))
}

fn grounding_user_text(args: &Value) -> String {
    let mut text = [
        "task_spec",
        "contract_and_constraints",
        "invariants_and_rules",
        "audit_focus",
        "objective",
        "problem_spec",
        "prompt",
        "custom_instructions",
        "context_notes",
        "design_rationale",
    ]
    .into_iter()
    .filter_map(|key| args[key].as_str())
    .collect::<Vec<_>>()
    .join("\n");
    for key in [
        "target_code_or_context",
        "target_code_or_diff",
        "target_code",
        "error_trace",
        "previous_attempts",
        "context_types",
        "signature",
    ] {
        let Some(context) = args[key].as_str() else {
            continue;
        };
        let source = (context.trim().starts_with('@')
            && context.split_whitespace().count() == 1
            && !context.contains(['/', '.', '"', '\'']))
            || context.lines().any(|line| {
                let line = line.trim_start();
                [
                    "fn ",
                    "pub fn ",
                    "async fn ",
                    "def ",
                    "class ",
                    "struct ",
                    "enum ",
                    "trait ",
                    "import ",
                    "from ",
                    "use ",
                    "+++ ",
                    "--- ",
                    "SELECT ",
                    "select ",
                    "INSERT ",
                    "UPDATE ",
                    "DELETE ",
                    "CREATE ",
                    "WITH ",
                ]
                .iter()
                .any(|prefix| line.starts_with(prefix))
                    || (line.starts_with('@') && context.lines().count() > 1)
            });
        // Source annotations are not attachments. Diff discovery still reads
        // these fenced bodies and includes actual indexed definition ranges.
        if source {
            text.push_str("\n```source\n");
        } else {
            text.push('\n');
        }
        text.push_str(context);
        if source {
            text.push_str("\n```\n");
        }
    }
    text
}

fn source_selection_text(
    text: &str,
    skills: &mut SkillRegistry,
) -> crate::Result<(String, Vec<String>)> {
    let marker = ACTIVE_SKILLS_MARKER
        .as_ref()
        .ok_or("active skills marker regex unavailable")?;
    let mut selected_text = text.to_owned();
    let mut verified_blocks = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for capture in marker.captures_iter(text).take(32) {
        if !seen.insert(capture[1].to_owned()) {
            continue;
        }
        let names = capture[1].split(',').map(str::trim).collect::<Vec<_>>();
        if names.is_empty()
            || names.len() > 32
            || names
                .iter()
                .any(|name| !skills.skills().contains_key(*name))
        {
            continue;
        }
        let mut registered = skills.clone();
        for name in registered.active().to_vec() {
            registered.deactivate(&name);
        }
        for name in &names {
            registered.activate(name)?;
        }
        let exact_body = registered.active_context(32_000)?;
        // A literal marker in user input has no authority. Exclude only a full
        // block matching the registered on-disk bodies, and retain text after it.
        if selected_text.contains(&exact_body) {
            selected_text = selected_text.replace(
                &exact_body,
                "[Registered instructions supplied separately; omitted from source selection.]",
            );
            for name in names {
                skills.activate(name)?;
            }
            verified_blocks.push(exact_body);
        }
    }
    Ok((selected_text, verified_blocks))
}

fn strip_verified_skill_blocks(value: &mut Value, verified_blocks: &[String]) {
    if verified_blocks.is_empty() {
        return;
    }
    match value {
        Value::String(text) => {
            for block in verified_blocks {
                if text.contains(block) {
                    *text = text.replace(
                        block,
                        "[Registered instructions supplied in Actual skill bodies section below.]",
                    );
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_verified_skill_blocks(item, verified_blocks);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                strip_verified_skill_blocks(item, verified_blocks);
            }
        }
        _ => {}
    }
}

fn normalized_source_arguments(args: &Value, root: &Path, user_text: &str) -> crate::Result<Value> {
    let builder = SourceContextBuilder::new(root)?;
    let requested = args["files_inspected"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let mut files = builder.normalized_files(&requested)?;
    files.extend(builder.referenced_paths(user_text)?);
    let mut seen = std::collections::BTreeSet::new();
    files.retain(|path| seen.insert(path.clone()));
    let mut args = args.clone();
    args["files_inspected"] = serde_json::to_value(files)?;
    Ok(args)
}

pub(super) fn grounded_prompt(
    name: &str,
    args: &Value,
    root: &Path,
) -> crate::Result<(Value, String)> {
    let home = std::env::var_os("HCODEX_HOME")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);
    let mut skills = SkillRegistry::discover(root, home.as_deref())?;
    let (user_text, verified_blocks) =
        source_selection_text(&grounding_user_text(args), &mut skills)?;
    let normalized_args = normalized_source_arguments(args, root, &user_text)?;
    let _ = skills.extract_mentions_where(&user_text, |token| token.starts_with('$'));
    let args = &normalized_args;
    let mut prompt_args = normalized_args.clone();
    strip_verified_skill_blocks(&mut prompt_args, &verified_blocks);
    let mut files = args["files_inspected"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let relevance = crate::context::discover_relevance(root, &user_text, 8)?;
    let explicit_files = !files.is_empty();
    if !explicit_files {
        files = relevance.files.clone();
    }
    let mut symbols = args["symbols_inspected"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if symbols.is_empty() && !explicit_files {
        symbols.extend(relevance.symbols.iter().cloned());
    }
    symbols.sort();
    symbols.dedup();
    let context_symbols: &[String] = if args.get("patch_contract").is_some() {
        &[]
    } else {
        &symbols
    };
    let code = SourceContextBuilder::new(root)?.build_with_text(
        &files,
        context_symbols,
        &user_text,
        64_000,
    )?;
    let prompt = format!(
        "{}\n\nTool: {name}\nTask and typed arguments:\n{}\nActual skill bodies:\n{}\n[CodeGraph Sliced Symbol Context]:\n{}\nSource selection rationale:\n{}\nReturn an explicit PASS/WARN/BLOCK verdict where reviewing. Execution evidence must come from executed tools; never invent success. Subagents may be used only when exposed by your runtime.",
        prompt_prefix(name, &prompt_args),
        serde_json::to_string_pretty(&prompt_args)?,
        skills.active_context(32_000)?,
        code.markdown,
        serde_json::to_string_pretty(&relevance)?
    );
    Ok((normalized_args, prompt))
}

pub(super) fn build_route_request(
    name: &str,
    args: &Value,
    prompt: String,
) -> crate::Result<RouteRequest> {
    let engine = if name == "claude_code_craft" {
        Engine::AntigravityRpc
    } else {
        match optional(args, "engine").as_deref().unwrap_or("cli") {
            "cli" | "codex_cli" | "codex-cli" => Engine::CodexCli,
            "antigravity_local_rpc" | "antigravity" | "agy" | "rpc" | "antigravity-rpc" => {
                Engine::AntigravityRpc
            }
            "http" | "local_http" | "openai-http" => Engine::Http,
            other => return Err(format!("unsupported execution engine: {other}").into()),
        }
    };
    let model = optional(args, "claude_model")
        .or_else(|| optional(args, "model"))
        .unwrap_or_else(|| {
            if engine == Engine::CodexCli {
                "gpt-6-astra".to_owned()
            } else if engine == Engine::Http {
                "gpt-6.1-sol".to_owned()
            } else {
                "claude-sonnet-5-5-medium".to_owned()
            }
        });
    let effort = optional(args, "reasoning_effort").unwrap_or_else(|| {
        if model.starts_with("claude-") {
            "medium".to_owned()
        } else {
            "xhigh".to_owned()
        }
    });
    let provider = if let Some(value) = optional(args, "provider") {
        serde_json::from_value(Value::String(value))?
    } else {
        match engine {
            Engine::CodexCli => Provider::OpenaiCodex,
            Engine::AntigravityRpc => Provider::Antigravity,
            Engine::Http => Provider::LocalHttp,
        }
    };
    Ok(RouteRequest {
        provider,
        engine,
        model,
        effort,
        prompt,
    })
}

#[cfg(test)]
pub(super) fn fallback_source_files(root: &Path) -> crate::Result<Vec<PathBuf>> {
    let allowed = ["rs", "py", "ts", "tsx", "js", "jsx"];
    let mut files = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            !matches!(
                entry.file_name().to_str(),
                Some(
                    ".git"
                        | ".venv"
                        | "target"
                        | "node_modules"
                        | ".codegraph"
                        | ".agents"
                        | ".omx"
                        | ".codex"
                )
            )
        })
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| allowed.contains(&extension))
        })
        .filter(|entry| {
            entry
                .metadata()
                .is_ok_and(|metadata| metadata.len() <= 16_000)
        })
        .filter_map(|entry| entry.path().strip_prefix(root).ok().map(Path::to_path_buf))
        .collect::<Vec<_>>();
    files.sort();
    files.truncate(6);
    Ok(files)
}

fn boundary_contract(name: &str) -> Option<Value> {
    let name = if name == "argon_architecture_review" {
        "argon_review"
    } else {
        name
    };
    BOUNDARY_SCHEMAS.as_ref()?.get(name).cloned()
}

pub(super) fn boundary_schema(name: &str) -> Option<Value> {
    boundary_contract(name).and_then(|contract| contract.get("request").cloned())
}

pub(super) fn normalize_boundary(name: &str, output: &mut Value) -> crate::Result<()> {
    let Some(contract) = boundary_contract(name) else {
        return Ok(());
    };
    crate::catalog::validate_schema(output, &contract["validation"])?;
    if let Some(object) = output.as_object_mut()
        && let Some(defaults) = contract["defaults"].as_object()
    {
        for (key, value) in defaults {
            object.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    Ok(())
}
