//! Typed command line adapter for the Rust Heterarchy Codex service.
use clap::{Parser, Subcommand};
use hcodex::{Result, gateway::Gateway};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "hcodex",
    version,
    about = "Rust Heterarchy Codex MCP and DAG workbench"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[arg(long, global = true, default_value = ".")]
    workspace: PathBuf,
    #[arg(long, global = true, env = "HCODEX_STATE_ROOT")]
    state_root: Option<PathBuf>,
    /// Create/reopen a named workspace; DAG mode uses independent node worktrees.
    #[arg(long, global = true)]
    worktree_name: Option<String>,
}
#[derive(Subcommand)]
enum Command {
    Mcp,
    #[command(alias = "chat")]
    Tui {
        #[arg(short = 'c', long = "command")]
        input: Option<String>,
        #[arg(
            trailing_var_arg = true,
            conflicts_with = "input",
            allow_hyphen_values = true
        )]
        initial: Vec<String>,
    },
    #[command(alias = "st")]
    Status,
    #[command(alias = "job")]
    Jobs,
    #[command(alias = "tickets")]
    Inbox,
    /// Show persisted campaigns, node handoffs, and review state without model calls.
    Board {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Run the existing three-role Review Council for one proposal.
    Council {
        /// Bind this review to a current workspace file; repeat for multiple files.
        #[arg(long = "file")]
        files: Vec<PathBuf>,
        #[arg(trailing_var_arg = true, required = true)]
        proposal: Vec<String>,
    },
    /// Read the actual role ballots and scoped source freshness for a council run.
    Votes {
        collaboration_id: String,
    },
    /// Inspect existing recovery evidence without changing jobs or worktrees.
    Recovery {
        job_id: String,
    },
    /// Continue a persisted DAG from its verified checkpoint frontier.
    Resume {
        dag_id: String,
    },
    #[command(alias = "prompt")]
    Ask {
        #[arg(trailing_var_arg = true, required = true)]
        prompt: Vec<String>,
    },
    Sonnet {
        #[arg(trailing_var_arg = true, required = true)]
        prompt: Vec<String>,
    },
    Opus {
        #[arg(trailing_var_arg = true, required = true)]
        prompt: Vec<String>,
    },
    Argon {
        #[arg(trailing_var_arg = true, required = true)]
        prompt: Vec<String>,
    },
    Flash {
        #[arg(trailing_var_arg = true, required = true)]
        prompt: Vec<String>,
    },
    #[command(name = "dag-ulw", alias = "dag")]
    Dag {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        goal: Vec<String>,
    },
    #[command(name = "dag-run", alias = "run")]
    DagRun {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        goal: Vec<String>,
    },
    DagMode {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        arguments: Vec<String>,
    },
    DagExit,
    #[command(aliases=["lanes","model"])]
    Models,
    #[command(alias = "skill")]
    Skills {
        #[arg(trailing_var_arg = true)]
        arguments: Vec<String>,
    },
    #[command(aliases=["enable","start"])]
    On,
    #[command(aliases=["disable","stop"])]
    Off,
    #[command(alias = "reload")]
    Restart,
    #[command(alias = "install")]
    Setup,
    #[command(alias = "refresh")]
    Sync,
    #[command(aliases=["compact","clean"])]
    Prune {
        #[arg(long, default_value_t = 1000)]
        retain: usize,
    },
}
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let home = PathBuf::from(
        std::env::var_os("HCODEX_HOME")
            .or_else(|| std::env::var_os("HOME"))
            .ok_or("home directory unavailable")?,
    );
    let state_root = cli
        .state_root
        .unwrap_or_else(|| home.join(".cache/codex-collab-mcp"));
    let gateway = Gateway::new(state_root.clone());
    let workspace = cli.workspace.canonicalize()?;
    let initial_existing_session = if cli.worktree_name.is_some() {
        let input = match &cli.command {
            Some(Command::Tui { input, initial }) if input.is_some() || !initial.is_empty() => {
                Some(input.clone().unwrap_or_else(|| initial.join(" ")))
            }
            Some(Command::Dag { goal }) => Some(format!("/dag-ulw {}", goal.join(" "))),
            Some(Command::DagRun { goal }) => Some(format!("/dag-run {}", goal.join(" "))),
            Some(Command::DagMode { arguments }) => {
                Some(format!("/dag-mode {}", arguments.join(" ")))
            }
            _ => None,
        };
        if let Some(input) = input {
            let workspace = workspace.clone();
            let home = home.clone();
            tokio::task::spawn_blocking(move || {
                hcodex::terminal_adapter::initial_existing_session_input(
                    &workspace,
                    Some(&home),
                    &input,
                )
            })
            .await??
        } else {
            false
        }
    } else {
        false
    };
    if cli.worktree_name.is_some()
        && (initial_existing_session
            || matches!(
                &cli.command,
                Some(
                    Command::Mcp
                        | Command::Setup
                        | Command::Sync
                        | Command::On
                        | Command::Off
                        | Command::Restart
                        | Command::Prune { .. }
                        | Command::Recovery { .. }
                        | Command::Resume { .. }
                )
            ))
    {
        return Err(
            "--worktree-name cannot start recovery/resume or server/installation management; use --workspace with an existing session"
                .into(),
        );
    }
    let named = if let Some(name) = &cli.worktree_name {
        Some(hcodex::dag::open_named_worktree(&workspace, &state_root, name).await?)
    } else {
        None
    };
    let worktree_mode = named.is_some();
    let workspace = if let Some(named) = &named {
        eprintln!("Named worktree '{}': {}", named.name, named.path.display());
        named.path.clone()
    } else {
        workspace
    };
    let command = cli.command.unwrap_or_else(|| {
        if worktree_mode {
            Command::Tui {
                input: None,
                initial: Vec::new(),
            }
        } else {
            Command::Status
        }
    });
    let terminal = |workspace, state_root, input| {
        terminal_command(workspace, state_root, input, worktree_mode)
    };
    let output=match command {
        Command::Mcp=>{
            let executable=std::env::current_exe()?;
            let owned=tokio::task::spawn_blocking(move||hcodex::lifecycle::OwnedMcpProcessGuard::register(&home,&executable)).await??;
            let outcome=hcodex::mcp::run(gateway).await;
            tokio::task::spawn_blocking(move||owned.unregister()).await??;
            return outcome;
        }
        Command::Tui{input:None,initial} if initial.is_empty()=>return if worktree_mode {hcodex::terminal_adapter::run_with_worktrees(workspace,state_root).await}else{hcodex::terminal_adapter::run(workspace,state_root).await},
        Command::Tui{input,initial}=>terminal(workspace,state_root,input.unwrap_or_else(||initial.join(" "))).await?,
        Command::DagMode{arguments}=>terminal(workspace,state_root,format!("/dag-mode {}",arguments.join(" "))).await?,
        Command::DagExit=>serde_json::to_value(hcodex::terminal_adapter::command(workspace,state_root,"/dag-exit".into()).await?)?,
        Command::On=>hcodex::lifecycle::control("on",&workspace,&home).await?,
        Command::Off=>hcodex::lifecycle::control("off",&workspace,&home).await?,
        Command::Restart=>hcodex::lifecycle::control("restart",&workspace,&home).await?,
        Command::Setup=>hcodex::lifecycle::control("setup",&workspace,&home).await?,
        Command::Sync=>hcodex::lifecycle::control("sync",&workspace,&home).await?,
        Command::Status=>gateway.call("codex_bridge_health",json!({"workspace_dir":workspace})).await?,
        Command::Jobs=>gateway.call("codex_job_list",json!({"workspace_dir":workspace})).await?,
        Command::Inbox=>gateway.call("heterarchy_handoff_inbox",json!({"workspace_dir":workspace})).await?,
        Command::Board{limit}=>hcodex::workbench::board(&gateway,&workspace,limit).await?,
        Command::Council{files,proposal}=>gateway.call("heterarchy_triad_orchestrate",hcodex::workbench::council_request_with_files(&proposal.join(" "),&workspace,&files)?).await?,
        Command::Votes{collaboration_id}=>hcodex::workbench::votes(&gateway,&workspace,&collaboration_id).await?,
        Command::Recovery{job_id}=>hcodex::workbench::recovery(&state_root,&workspace,&job_id).await?,
        Command::Resume{dag_id}=>terminal(workspace,state_root,format!("/resume {dag_id}")).await?,
        Command::Ask{prompt}|Command::Argon{prompt}=>terminal(workspace,state_root,format!("/argon {}",prompt.join(" "))).await?,
        Command::Sonnet{prompt}=>terminal(workspace,state_root,format!("/sonnet {}",prompt.join(" "))).await?,
        Command::Opus{prompt}=>terminal(workspace,state_root,format!("/opus {}",prompt.join(" "))).await?,
        Command::Flash{prompt}=>terminal(workspace,state_root,format!("/flash {}",prompt.join(" "))).await?,
        Command::Dag{goal}=>terminal(workspace,state_root,format!("/dag-ulw {}",campaign_goal(goal))).await?,
        Command::DagRun{goal}=>terminal(workspace,state_root,format!("/dag-run {}",campaign_goal(goal))).await?,
        Command::Prune{retain}=>gateway.database(move|store|Ok(json!({"pruned_jobs":store.prune_completed_jobs(retain,true)?,"pruned_handoffs":store.prune_resolved_handoffs(retain,true)?}))).await?,
        Command::Models=>terminal(workspace,state_root,"/models".to_owned()).await?,
        Command::Skills{arguments}=>terminal(workspace,state_root,format!("/skills {}",arguments.join(" "))).await?,
    };
    gateway.shutdown().await?;
    let output = if let Some(named) = named {
        json!({"named_worktree":named,"result":output})
    } else {
        output
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
async fn terminal_command(
    workspace: PathBuf,
    state_root: PathBuf,
    input: String,
    worktree: bool,
) -> Result<Value> {
    Ok(serde_json::to_value(if worktree {
        hcodex::terminal_adapter::command_with_worktrees(workspace, state_root, input).await?
    } else {
        hcodex::terminal_adapter::command(workspace, state_root, input).await?
    })?)
}
fn campaign_goal(goal: Vec<String>) -> String {
    if goal.is_empty() {
        "Verify and refactor module contracts".to_owned()
    } else {
        goal.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_worktree_flag_preserves_default_and_explicit_terminal_commands() -> Result<()> {
        let cli = Cli::try_parse_from(["hcodex", "--worktree-name", "feature-auth"])?;
        assert_eq!(cli.worktree_name.as_deref(), Some("feature-auth"));
        assert!(cli.command.is_none());
        let cli = Cli::try_parse_from([
            "hcodex",
            "--worktree-name",
            "feature-auth",
            "dag-run",
            "change",
            "module",
        ])?;
        assert_eq!(cli.worktree_name.as_deref(), Some("feature-auth"));
        assert!(matches!(cli.command,Some(Command::DagRun {goal}) if goal==["change","module"]));
        assert!(Cli::try_parse_from(["hcodex"])?.worktree_name.is_none());
        Ok(())
    }

    #[test]
    fn historical_cli_forms_preserve_arguments_and_explicit_plan_only() -> Result<()> {
        assert!(
            matches!(Cli::try_parse_from(["hcodex","tui","/skills","core"])?.command,
            Some(Command::Tui{input:None,initial}) if initial==["/skills","core"])
        );
        assert!(
            matches!(Cli::try_parse_from(["hcodex","tui","-c","/skills core"])?.command,
            Some(Command::Tui{input:Some(input),..}) if input=="/skills core")
        );
        assert!(
            matches!(Cli::try_parse_from(["hcodex","skills","core"])?.command,
            Some(Command::Skills{arguments}) if arguments==["core"])
        );
        assert!(
            matches!(Cli::try_parse_from(["hcodex","dag-mode","off"])?.command,
            Some(Command::DagMode{arguments}) if arguments==["off"])
        );
        assert!(
            matches!(Cli::try_parse_from(["hcodex","dag-ulw","--plan-only","audit"])?.command,
            Some(Command::Dag{goal}) if goal==["--plan-only","audit"])
        );
        assert!(matches!(Cli::try_parse_from(["hcodex","dag-run"])?.command,
            Some(Command::DagRun{goal}) if goal.is_empty()));
        assert!(matches!(
            Cli::try_parse_from(["hcodex", "board", "--limit", "7"])?.command,
            Some(Command::Board { limit: 7 })
        ));
        assert!(
            matches!(Cli::try_parse_from(["hcodex","council","review","handoff"])?.command,
            Some(Command::Council{files,proposal}) if files.is_empty() && proposal==["review","handoff"])
        );
        assert!(
            matches!(Cli::try_parse_from(["hcodex","council","--file","src/lib.rs","--file","README.md","review","handoff"])?.command,
            Some(Command::Council{files,proposal}) if files==[PathBuf::from("src/lib.rs"),PathBuf::from("README.md")] && proposal==["review","handoff"])
        );
        assert!(
            matches!(Cli::try_parse_from(["hcodex","votes","cdx-review-1"])?.command,
            Some(Command::Votes{collaboration_id}) if collaboration_id=="cdx-review-1")
        );
        assert!(matches!(
            Cli::try_parse_from(["hcodex", "recovery", "cdx-campaign-1"])?.command,
            Some(Command::Recovery { job_id }) if job_id == "cdx-campaign-1"
        ));
        assert!(Cli::try_parse_from(["hcodex", "recovery"]).is_err());
        assert!(Cli::try_parse_from(["hcodex", "recovery", "one", "two"]).is_err());
        assert_eq!(
            campaign_goal(Vec::new()),
            "Verify and refactor module contracts"
        );
        Ok(())
    }
}
