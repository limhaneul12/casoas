//! Shared CLI/TUI argument adapter for the read-only research workflow.

use std::io::Read;
use std::path::{Path, PathBuf};

use clap::{Args, Parser};
use serde_json::Value;

use crate::Result;
use crate::gateway::Gateway;
use crate::research::{
    Audience, DiscoveryConfig, Investigator, ModelSpec, ResearchRequest, SourceSpec,
};

const MAX_REQUEST_BYTES: u64 = 1024 * 1024;

#[derive(Args, Clone, Debug, Default)]
pub struct ResearchArgs {
    /// Repository file relative to --workspace; repeat for multiple sources.
    #[arg(long = "file")]
    pub files: Vec<PathBuf>,
    /// Local technical document relative to --workspace.
    #[arg(long = "document")]
    pub documents: Vec<PathBuf>,
    /// Explicit public HTTPS source.
    #[arg(long = "url")]
    pub urls: Vec<String>,
    /// Run bounded native web discovery, including when explicit sources are supplied.
    #[arg(long, conflicts_with = "no_web")]
    pub web: bool,
    /// Disable automatic web discovery for a question-only request.
    #[arg(long, conflicts_with_all = ["web", "search_model"])]
    pub no_web: bool,
    /// Discovery model; specifying it enables bounded web discovery.
    #[arg(long)]
    pub search_model: Option<String>,
    /// Show the source and stage plan without network, provider or storage effects.
    #[arg(long)]
    pub plan_only: bool,
    /// Exact durable identity; an existing request is read back without redispatch.
    #[arg(long)]
    pub request_id: Option<String>,
    #[arg(long)]
    pub idempotency_key: Option<String>,
    /// Closed JSON ResearchRequest. Its workspace must match --workspace.
    #[arg(long, conflicts_with_all = ["files", "documents", "urls", "web", "no_web", "search_model", "request_id", "idempotency_key", "investigators", "challenger", "synthesizer", "audience", "language", "as_of", "timeout_seconds", "max_sources", "max_parallel", "question"])]
    pub request_file: Option<PathBuf>,
    /// Full investigator model ID; repeat up to three times. JSON exposes focus/effort.
    #[arg(long = "investigator")]
    pub investigators: Vec<String>,
    #[arg(long)]
    pub challenger: Option<String>,
    #[arg(long)]
    pub synthesizer: Option<String>,
    #[arg(long, value_parser = ["human", "coding_agent"])]
    pub audience: Option<String>,
    #[arg(long)]
    pub language: Option<String>,
    /// Historical constraint; v1 current captures cannot prove historical validity.
    #[arg(long)]
    pub as_of: Option<String>,
    #[arg(long)]
    pub timeout_seconds: Option<u64>,
    #[arg(long)]
    pub max_sources: Option<usize>,
    #[arg(long)]
    pub max_parallel: Option<usize>,
    #[arg(trailing_var_arg = true)]
    pub question: Vec<String>,
}

#[derive(Parser)]
#[command(name = "research", disable_help_subcommand = true)]
struct ResearchCommand {
    #[command(flatten)]
    arguments: ResearchArgs,
}

impl ResearchArgs {
    /// TUI words follow the existing simple slash-command convention. Paths
    /// containing whitespace should use the native CLI, which preserves quoting.
    pub fn parse_tui(arguments: &str) -> Result<Self> {
        let words = std::iter::once("research").chain(arguments.split_whitespace());
        Ok(ResearchCommand::try_parse_from(words)?.arguments)
    }

    pub async fn into_request(self, workspace: PathBuf) -> Result<ResearchRequest> {
        let workspace = workspace.canonicalize()?;
        if let Some(path) = self.request_file {
            let selected_workspace = workspace.clone();
            let mut request =
                tokio::task::spawn_blocking(move || read_request(&path, &selected_workspace))
                    .await??;
            request.plan_only |= self.plan_only;
            return Ok(request);
        }
        let question = self.question.join(" ");
        if question.trim().is_empty() {
            return Err(
                "research question is required; use research --help for source options".into(),
            );
        }
        let mut request = ResearchRequest::new(question, workspace);
        request.sources.extend(
            self.files
                .into_iter()
                .map(|path| SourceSpec::Repository { path }),
        );
        request.sources.extend(
            self.documents
                .into_iter()
                .map(|path| SourceSpec::TechnicalDocument { path }),
        );
        request
            .sources
            .extend(self.urls.into_iter().map(|url| SourceSpec::Web { url }));
        request.discovery = if self.no_web {
            Some(DiscoveryConfig::Disabled)
        } else if let Some(model) = self.search_model {
            Some(DiscoveryConfig::web_with_model(model))
        } else if self.web {
            Some(DiscoveryConfig::default_web())
        } else {
            None
        };
        if !self.investigators.is_empty() {
            request.investigators = self
                .investigators
                .into_iter()
                .enumerate()
                .map(|(index, model)| Investigator {
                    lane_id: format!("researcher_{}", index + 1),
                    focus: format!("Independent evidence assessment {}", index + 1),
                    effort: model_effort(&model).to_owned(),
                    model,
                })
                .collect();
        }
        if let Some(model) = self.challenger {
            request.challenger = model_spec(model);
        }
        if let Some(model) = self.synthesizer {
            request.synthesizer = model_spec(model);
        }
        if let Some(audience) = self.audience {
            request.audience = match audience.as_str() {
                "human" => Audience::Human,
                "coding_agent" => Audience::CodingAgent,
                _ => return Err("unsupported research audience".into()),
            };
        }
        if let Some(language) = self.language {
            request.output_language = language;
        }
        request.as_of = self.as_of;
        request.plan_only = self.plan_only;
        request.request_id = self.request_id;
        request.idempotency_key = self.idempotency_key;
        if let Some(timeout) = self.timeout_seconds {
            request.budget.timeout_seconds = timeout;
        }
        if let Some(limit) = self.max_sources {
            request.budget.max_sources = limit;
        }
        if let Some(limit) = self.max_parallel {
            request.budget.max_parallel = limit;
        }
        Ok(request)
    }
}

pub(crate) fn model_spec(model: String) -> ModelSpec {
    ModelSpec {
        effort: model_effort(&model).to_owned(),
        model,
    }
}

fn model_effort(model: &str) -> &'static str {
    if model.starts_with("claude-") {
        "medium"
    } else {
        "high"
    }
}

fn read_request(path: &Path, workspace: &Path) -> Result<ResearchRequest> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("research request file must be a regular file".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_REQUEST_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err("research request file exceeds 1 MiB".into());
    }
    let mut request: ResearchRequest = serde_json::from_slice(&bytes)?;
    if request.workspace_dir.canonicalize()? != workspace {
        return Err("research request workspace differs from --workspace".into());
    }
    request.workspace_dir = workspace.to_path_buf();
    Ok(request)
}

/// Ctrl-C sets a latch created before admission and then drains the same owned
/// future. It neither drops in-flight work nor starts a replacement request.
pub async fn run_foreground(gateway: &Gateway, mut request: ResearchRequest) -> Result<Value> {
    if !request.plan_only && (!request.sources.is_empty() || request.active_discovery().is_some()) {
        let request_id = request
            .request_id
            .get_or_insert_with(|| format!("research-{}", uuid::Uuid::new_v4().simple()));
        eprintln!("Research request: {}", request_id.escape_debug());
    }
    let (cancel, receiver) = tokio::sync::watch::channel(false);
    let run = crate::research::run_with_cancellation(gateway, request, receiver);
    tokio::pin!(run);
    tokio::select! {
        // Install and poll the signal listener before first admission.
        biased;
        signal = tokio::signal::ctrl_c() => {
            let _sent = cancel.send(true);
            let result = run.await;
            signal?;
            result
        },
        result = &mut run => result,
    }
}

/// Render only the host's source-linked Markdown and observation metadata.
/// Model advisory prose and raw provider payloads are never a report fallback.
pub fn report_lines(value: &Value) -> Vec<String> {
    let report = value
        .get("report")
        .filter(|report| report.is_object())
        .unwrap_or(value);
    let mut lines = Vec::new();
    for key in [
        "request_id",
        "job_id",
        "job_status",
        "status",
        "research_status",
        "error",
    ] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            lines.push(format!("{key}: {text}"));
        }
    }
    if let Some(markdown) = report.get("markdown").and_then(Value::as_str) {
        lines.extend(markdown.lines().map(str::to_owned));
    } else {
        lines.push("No completed source-linked report is available.".to_owned());
        if value.get("plan_only") == Some(&Value::Bool(true))
            || value.get("plan").is_some()
            || value.get("missing_inputs").is_some()
        {
            lines.push("Plan only: automatic web discovery or explicit --file, --document and public --url sources can execute the study.".to_owned());
        }
        if let Some(missing) = value.get("missing_inputs").and_then(Value::as_array) {
            lines.extend(missing.iter().filter_map(Value::as_str).map(str::to_owned));
        }
    }
    lines
        .into_iter()
        .map(|line| {
            line.chars()
                .filter(|character| !character.is_control() || *character == '\t')
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cli_and_tui_preserve_source_kinds_and_model_selection() -> Result<()> {
        let workspace = tempfile::tempdir()?;
        let args = ResearchArgs::parse_tui(
            "--file code.rs --document guide.md --url https://example.com/spec --investigator gemini-4-argon-high --investigator gemini-4-argon-high --challenger claude-opus-5-5-medium --plan-only -- compare source contracts",
        )?;
        let request = args.into_request(workspace.path().to_path_buf()).await?;
        assert_eq!(request.question, "compare source contracts");
        assert!(matches!(request.sources[0], SourceSpec::Repository { .. }));
        assert!(matches!(
            request.sources[1],
            SourceSpec::TechnicalDocument { .. }
        ));
        assert!(matches!(request.sources[2], SourceSpec::Web { .. }));
        assert_eq!(request.investigators.len(), 2);
        assert_ne!(
            request.investigators[0].lane_id,
            request.investigators[1].lane_id
        );
        assert_eq!(
            request.investigators[0].model,
            request.investigators[1].model
        );
        assert_eq!(request.challenger.effort, "medium");
        assert!(request.plan_only);
        assert!(ResearchArgs::parse_tui("--write target.rs -- question").is_err());
        Ok(())
    }

    #[tokio::test]
    async fn discovery_flags_are_closed_and_question_only_defaults_lazily() -> Result<()> {
        let workspace = tempfile::tempdir()?;
        let question_only = ResearchArgs::parse_tui("-- investigate source contracts")?
            .into_request(workspace.path().to_path_buf())
            .await?;
        assert!(question_only.discovery.is_none());
        assert_eq!(
            question_only.active_discovery(),
            Some(DiscoveryConfig::default_web())
        );
        assert!(
            !serde_json::to_value(&question_only)?
                .as_object()
                .ok_or("request object")?
                .contains_key("discovery")
        );
        let round_trip: ResearchRequest =
            serde_json::from_value(serde_json::to_value(&question_only)?)?;
        assert!(round_trip.discovery.is_none());

        let seeded = ResearchArgs::parse_tui("--url https://example.com -- inspect")?
            .into_request(workspace.path().to_path_buf())
            .await?;
        assert!(seeded.active_discovery().is_none());
        assert!(
            !serde_json::to_value(&seeded)?
                .as_object()
                .ok_or("request object")?
                .contains_key("discovery")
        );

        let forced = ResearchArgs::parse_tui(
            "--url https://example.com --search-model gpt-6.1-sol -- inspect",
        )?
        .into_request(workspace.path().to_path_buf())
        .await?;
        assert!(matches!(
            forced.discovery,
            Some(DiscoveryConfig::Web { ref model, .. }) if model == "gpt-6.1-sol"
        ));

        let disabled = ResearchArgs::parse_tui("--no-web -- inspect")?
            .into_request(workspace.path().to_path_buf())
            .await?;
        assert_eq!(disabled.discovery, Some(DiscoveryConfig::Disabled));
        assert!(disabled.active_discovery().is_none());
        assert!(ResearchArgs::parse_tui("--web --no-web -- inspect").is_err());
        assert!(ResearchArgs::parse_tui("--no-web --search-model gpt-6.1-sol -- inspect").is_err());

        let defaulted: DiscoveryConfig = serde_json::from_value(serde_json::json!({"kind":"web"}))?;
        assert_eq!(defaulted, DiscoveryConfig::default_web());
        assert!(
            serde_json::from_value::<DiscoveryConfig>(
                serde_json::json!({"kind":"web","unexpected":true})
            )
            .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn closed_request_file_binds_selected_workspace_and_rejects_overrides() -> Result<()> {
        let workspace = tempfile::tempdir()?;
        let other = tempfile::tempdir()?;
        let path = workspace.path().join("request.json");
        let request = ResearchRequest::new("inspect contracts", workspace.path().to_path_buf());
        std::fs::write(&path, serde_json::to_vec(&request)?)?;
        let args = ResearchCommand::try_parse_from([
            "research",
            "--request-file",
            path.to_str().ok_or("path UTF-8")?,
            "--plan-only",
        ])?
        .arguments;
        assert!(
            args.into_request(workspace.path().to_path_buf())
                .await?
                .plan_only
        );
        assert!(
            ResearchCommand::try_parse_from([
                "research",
                "--request-file",
                path.to_str().ok_or("path UTF-8")?,
                "--file",
                "code.rs",
            ])
            .is_err()
        );
        let args = ResearchArgs {
            request_file: Some(path.clone()),
            ..ResearchArgs::default()
        };
        assert!(args.into_request(other.path().to_path_buf()).await.is_err());
        let mut invalid = serde_json::to_value(request)?;
        invalid["write_files"] = serde_json::json!(["secret.rs"]);
        std::fs::write(&path, serde_json::to_vec(&invalid)?)?;
        assert!(read_request(&path, workspace.path()).is_err());
        Ok(())
    }

    #[test]
    fn report_view_never_promotes_advisory_or_provider_text() {
        let value = serde_json::json!({"research_status":"partial","advisory_summary":"UNTRUSTED_ADVISORY","provider":{"output":"UNTRUSTED_PROVIDER"}});
        let lines = report_lines(&value).join("\n");
        assert!(lines.contains("partial"));
        assert!(!lines.contains("UNTRUSTED_ADVISORY"));
        assert!(!lines.contains("UNTRUSTED_PROVIDER"));
        assert!(
            report_lines(&serde_json::json!({"markdown":"# Report\n[source-1] observed quote"}))
                .join("\n")
                .contains("observed quote")
        );
        assert!(
            report_lines(&serde_json::json!({"markdown":"source\u{1b}[31m observation"}))
                .iter()
                .all(|line| !line.contains('\u{1b}'))
        );
    }
}
