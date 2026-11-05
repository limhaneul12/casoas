//! Pure argument adapter for the three TUI workspaces and explicit stage chains.
use std::path::PathBuf;

use clap::{Args, Parser};

use crate::Result;
use crate::workspaces::{WorkflowRequest, WorkspaceKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceCommand {
    Planning,
    Building,
    Workflow,
}

#[derive(Debug, Default, Args)]
struct WorkspaceArgs {
    /// Last stage of a Research -> Planning -> Building chain.
    #[arg(long, conflicts_with = "stages")]
    through: Option<WorkspaceKind>,
    /// Explicit ordered stages, for example planning,building.
    #[arg(long, value_delimiter = ',', conflicts_with = "through")]
    stages: Vec<WorkspaceKind>,
    /// Import one exact existing research campaign; never selects a global latest.
    #[arg(long = "from", alias = "research-id")]
    research_id: Option<String>,
    /// Import one exact persisted planning artifact for a Building-only request.
    #[arg(long = "plan")]
    plan_id: Option<String>,
    #[arg(long)]
    request_id: Option<String>,
    #[arg(long)]
    planner: Option<String>,
    #[arg(long)]
    planner_effort: Option<String>,
    #[arg(long)]
    timeout_seconds: Option<u64>,
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    goal: Vec<String>,
}

#[derive(Parser)]
#[command(name = "workspace", disable_help_subcommand = true)]
struct ParsedWorkspace {
    #[command(flatten)]
    args: WorkspaceArgs,
}

fn parse(arguments: &str) -> Result<WorkspaceArgs> {
    Ok(ParsedWorkspace::try_parse_from(
        std::iter::once("workspace").chain(arguments.split_whitespace()),
    )?
    .args)
}

fn selected(command: WorkspaceCommand, args: &WorkspaceArgs) -> Result<Vec<WorkspaceKind>> {
    use WorkspaceKind::{Building, Planning, Research};
    match command {
        WorkspaceCommand::Planning => {
            if args.through.is_some() || !args.stages.is_empty() || args.plan_id.is_some() {
                return Err(
                    "/planning accepts --from <research_id> and a goal; use /workflow for a chain"
                        .into(),
                );
            }
            Ok(vec![Planning])
        }
        WorkspaceCommand::Building => {
            if args.through.is_some() || !args.stages.is_empty() || args.research_id.is_some() {
                return Err("/building accepts --plan <plan_id>; use /workflow for a chain".into());
            }
            if args.plan_id.is_none() {
                return Err("usage: /building --plan <planning_artifact_id>".into());
            }
            Ok(vec![Building])
        }
        WorkspaceCommand::Workflow => {
            if !args.stages.is_empty() {
                return Ok(args.stages.clone());
            }
            match args.through.unwrap_or(Planning) {
                Research => Ok(vec![Research]),
                Planning => Ok(vec![Research, Planning]),
                Building => Ok(vec![Research, Planning, Building]),
            }
        }
    }
}

/// Useful to select a pane immediately from the operator's input, before any I/O.
pub fn selected_workspaces(
    command: WorkspaceCommand,
    arguments: &str,
) -> Result<Vec<WorkspaceKind>> {
    selected(command, &parse(arguments)?)
}

pub fn request(
    command: WorkspaceCommand,
    arguments: &str,
    original: PathBuf,
) -> Result<WorkflowRequest> {
    let args = parse(arguments)?;
    let stages = selected(command, &args)?;
    let goal = args.goal.join(" ");
    if goal.trim().is_empty() && stages != [WorkspaceKind::Building] {
        return Err("a research or planning goal is required after --".into());
    }
    let mut request = WorkflowRequest::new(goal, original, stages);
    request.request_id = args.request_id;
    request.research_id = args.research_id;
    request.plan_id = args.plan_id;
    if let Some(model) = args.planner {
        request.planner = crate::research_cli::model_spec(model);
    }
    if let Some(effort) = args.planner_effort {
        request.planner.effort = effort;
    }
    if let Some(timeout) = args.timeout_seconds {
        request.timeout_seconds = timeout;
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_and_chained_requests_preserve_explicit_build_intent() -> Result<()> {
        use WorkspaceKind::{Building, Planning, Research};
        let original = PathBuf::from("/original");
        assert_eq!(
            request(
                WorkspaceCommand::Planning,
                "--from research-original -- compare designs",
                original.clone()
            )?
            .stages,
            [Planning]
        );
        assert_eq!(
            request(
                WorkspaceCommand::Building,
                "--plan plan-original",
                original.clone()
            )?
            .stages,
            [Building]
        );
        assert_eq!(
            request(
                WorkspaceCommand::Workflow,
                "--through research -- investigate",
                original.clone()
            )?
            .stages,
            [Research]
        );
        assert_eq!(
            request(
                WorkspaceCommand::Workflow,
                "-- compare designs",
                original.clone()
            )?
            .stages,
            [Research, Planning]
        );
        assert_eq!(
            request(
                WorkspaceCommand::Workflow,
                "--through building -- develop feature",
                original.clone()
            )?
            .stages,
            [Research, Planning, Building]
        );
        let combined = request(
            WorkspaceCommand::Workflow,
            "--stages planning,building -- develop feature",
            original,
        )?;
        assert_eq!(combined.stages, [Planning, Building]);
        assert!(combined.request_id.is_none());
        assert!(
            request(
                WorkspaceCommand::Planning,
                "--through building -- forbidden",
                PathBuf::from("/original")
            )
            .is_err()
        );
        assert!(
            request(
                WorkspaceCommand::Building,
                "-- missing plan",
                PathBuf::from("/original")
            )
            .is_err()
        );
        assert!(
            selected_workspaces(
                WorkspaceCommand::Workflow,
                "--through planning --stages building -- invalid"
            )
            .is_err()
        );
        Ok(())
    }
}
