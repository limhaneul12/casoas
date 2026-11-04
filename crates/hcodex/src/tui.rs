//! Interactive terminal workbench for Heterarchy Codex.
//!
//! The module owns presentation and keyboard interaction. Model dispatch and DAG
//! execution stay behind [`ActionHandler`], so the UI never fabricates a model
//! response or an execution receipt.

mod completion;
mod model;
mod render;
mod text;

use std::future::Future;
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio::time::MissedTickBehavior;

pub use completion::{CompletionItem, CompletionKind, TuiCatalog};
pub use model::{
    DagLaneView, LaneState, TokenUsage, TranscriptEntry, TuiAction, TuiApp, TuiTheme, TuiUpdate,
    action_from_input,
};
pub use render::draw;

use crate::Result;

const MAX_ACTIVE_ACTIONS: usize = 8;
const TICK_INTERVAL: Duration = Duration::from_millis(16);

/// Boxed future returned by an action handler.
pub type ActionFuture = Pin<Box<dyn Future<Output = Result<()>> + Send>>;

/// Runtime boundary used by the TUI to dispatch typed user actions.
pub trait ActionHandler: Send + Sync + 'static {
    /// Dispatches one action and streams updates to the visible state.
    fn handle(&self, action: TuiAction, updates: mpsc::Sender<TuiUpdate>) -> ActionFuture;

    /// Cooperatively cancels and drains runtime-owned work before TUI teardown.
    fn shutdown(&self) -> ActionFuture {
        Box::pin(async { Ok(()) })
    }
}

impl<F, Fut> ActionHandler for F
where
    F: Fn(TuiAction, mpsc::Sender<TuiUpdate>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    fn handle(&self, action: TuiAction, updates: mpsc::Sender<TuiUpdate>) -> ActionFuture {
        Box::pin((self)(action, updates))
    }
}

/// Runs the terminal with an explicit runtime action boundary.
pub async fn run_with_handler<H>(workspace: PathBuf, handler: H) -> Result<()>
where
    H: ActionHandler,
{
    let catalog_workspace = workspace.clone();
    let catalog =
        tokio::task::spawn_blocking(move || TuiCatalog::discover(&catalog_workspace)).await??;
    let mut app = TuiApp::new(workspace, catalog);
    let mut terminal = TerminalGuard::enter()?;
    let handler = Arc::new(handler);
    let (event_tx, mut event_rx) = mpsc::channel::<Event>(32);
    let (update_tx, mut update_rx) = mpsc::channel::<TuiUpdate>(128);
    let stop_events = Arc::new(AtomicBool::new(false));
    let event_stop_guard = EventStopGuard(Arc::clone(&stop_events));
    let reader = tokio::task::spawn_blocking(move || read_events(event_tx, &stop_events));
    let mut actions = JoinSet::new();
    let mut tick = tokio::time::interval(TICK_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let loop_result: Result<()> = async {
        while !app.should_quit() {
            app.set_viewport_width(terminal.terminal.size()?.width);
            terminal.terminal.draw(|frame| draw(frame, &app))?;
            tokio::select! {
                _ = tick.tick() => app.tick(),
                maybe_update = update_rx.recv() => {
                    if let Some(update) = maybe_update {
                        app.apply_update(update);
                        // Consume one bounded burst before drawing again, keeping input responsive.
                        for _ in 1..32 {
                            let Ok(update) = update_rx.try_recv() else { break };
                            app.apply_update(update);
                        }
                    }
                }
                result = actions.join_next(), if !actions.is_empty() => {
                    if let Some(Err(join_error)) = result {
                        app.apply_update(TuiUpdate::Transcript(TranscriptEntry::root(
                            format!("Action task failed: {join_error}"),
                        )));
                    }
                }
                maybe_event = event_rx.recv() => {
                    let Some(input_event) = maybe_event else { break };
                    if let Some(action) = handle_input_event(&mut app, input_event)? {
                        if actions.len() >= MAX_ACTIVE_ACTIONS {
                            app.apply_update(TuiUpdate::Transcript(TranscriptEntry::root(
                                "Action capacity reached; wait for an active action to finish",
                            )));
                        } else {
                            spawn_action(
                                &mut actions,
                                Arc::clone(&handler),
                                update_tx.clone(),
                                action,
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }
    .await;

    let shutdown_result = shutdown_actions(&handler, &mut actions).await;
    drop(event_rx);
    event_stop_guard.0.store(true, Ordering::Release);
    let _ = reader.await;
    loop_result.and(shutdown_result)
}

/// Runs the TUI without an execution adapter.
///
/// Navigation and discovery remain available. Dispatch attempts produce an
/// explicit unavailable update, never a synthetic success or receipt.
pub async fn run(workspace: PathBuf) -> Result<()> {
    run_with_handler(
        workspace,
        |action: TuiAction, updates: mpsc::Sender<TuiUpdate>| async move {
            updates
                .send(TuiUpdate::RuntimeUnavailable {
                    action,
                    reason: "Rust runtime action adapter is not attached".to_owned(),
                })
                .await?;
            Ok(())
        },
    )
    .await
}

fn spawn_action<H>(
    actions: &mut JoinSet<()>,
    handler: Arc<H>,
    updates: mpsc::Sender<TuiUpdate>,
    action: TuiAction,
) where
    H: ActionHandler,
{
    actions.spawn(async move {
        if let Err(error) = handler.handle(action, updates.clone()).await {
            let _ = updates
                .send(TuiUpdate::Transcript(TranscriptEntry::root(format!(
                    "Action failed: {error}"
                ))))
                .await;
            let _ = updates
                .send(TuiUpdate::HeaderProgress { done: 0, total: 0 })
                .await;
        }
    });
}

async fn shutdown_actions<H>(handler: &Arc<H>, actions: &mut JoinSet<()>) -> Result<()>
where
    H: ActionHandler,
{
    let shutdown = tokio::time::timeout(Duration::from_secs(6), handler.shutdown()).await;
    let shutdown_result = match shutdown {
        Ok(result) => result,
        Err(_) => Err("action handler cooperative shutdown timed out".into()),
    };
    let drain = async {
        while let Some(joined) = actions.join_next().await {
            if let Err(error) = joined
                && !error.is_cancelled()
            {
                return Err(error.into());
            }
        }
        Ok::<(), crate::Error>(())
    };
    let drain_result = match tokio::time::timeout(Duration::from_secs(1), drain).await {
        Ok(result) => result,
        Err(_) => {
            actions.abort_all();
            while actions.join_next().await.is_some() {}
            Ok(())
        }
    };
    shutdown_result.and(drain_result)
}

fn read_events(sender: mpsc::Sender<Event>, stop: &AtomicBool) {
    while !stop.load(Ordering::Acquire) {
        match event::poll(Duration::from_millis(50)) {
            Ok(true) => match event::read() {
                Ok(input_event) => {
                    if sender.blocking_send(input_event).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            },
            Ok(false) => {}
            Err(_) => break,
        }
    }
}

fn handle_input_event(app: &mut TuiApp, input_event: Event) -> Result<Option<TuiAction>> {
    match input_event {
        Event::Key(key) if key.kind == KeyEventKind::Press => app.handle_key(key),
        Event::Paste(text) => {
            app.handle_paste(&text);
            Ok(None)
        }
        _ => Ok(None),
    }
}

struct EventStopGuard(Arc<AtomicBool>);

impl Drop for EventStopGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl TerminalGuard {
    fn enter() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableBracketedPaste) {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen);
            return Err(error.into());
        }
        let backend = CrosstermBackend::new(stdout);
        match Terminal::new(backend) {
            Ok(terminal) => Ok(Self { terminal }),
            Err(error) => {
                let _ = disable_raw_mode();
                let _ = execute!(io::stdout(), DisableBracketedPaste, LeaveAlternateScreen);
                Err(error.into())
            }
        }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
    }
}

pub(crate) fn is_quit_key(key: KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::path::PathBuf;
    #[cfg(unix)]
    use std::process::Stdio;
    #[cfg(unix)]
    use tokio::process::Command;
    #[cfg(unix)]
    use tokio::sync::watch;

    #[tokio::test]
    async fn action_updates_stream_before_task_completion() -> Result<()> {
        assert!(TICK_INTERVAL <= Duration::from_millis(16));
        let (updates, mut receiver) = mpsc::channel(4);
        let handler = Arc::new(
            |_action: TuiAction, updates: mpsc::Sender<TuiUpdate>| async move {
                updates
                    .send(TuiUpdate::Transcript(TranscriptEntry::root("first")))
                    .await?;
                tokio::time::sleep(Duration::from_millis(20)).await;
                updates
                    .send(TuiUpdate::Transcript(TranscriptEntry::root("second")))
                    .await?;
                Ok(())
            },
        );
        let mut actions = JoinSet::new();
        spawn_action(&mut actions, handler, updates, TuiAction::ShowStatus);

        let first = tokio::time::timeout(Duration::from_secs(1), receiver.recv()).await?;
        assert!(matches!(first, Some(TuiUpdate::Transcript(_))));
        assert!(!actions.is_empty());
        let _ = actions.join_next().await;
        let second = receiver.recv().await;
        assert!(matches!(second, Some(TuiUpdate::Transcript(_))));
        Ok(())
    }

    #[test]
    fn paste_event_inserts_once_and_never_submits() -> Result<()> {
        let mut app = TuiApp::new(PathBuf::from("."), TuiCatalog::default());
        let action =
            handle_input_event(&mut app, Event::Paste("line one\nline two\t🙂".to_owned()))?;
        assert!(action.is_none());
        assert_eq!(app.input(), "line one\nline two\t🙂");
        Ok(())
    }

    #[cfg(unix)]
    #[derive(Clone)]
    struct ProcessHandler {
        cancel: watch::Sender<bool>,
        pid_file: PathBuf,
    }

    #[cfg(unix)]
    impl ActionHandler for ProcessHandler {
        fn handle(&self, _action: TuiAction, _updates: mpsc::Sender<TuiUpdate>) -> ActionFuture {
            let mut cancel = self.cancel.subscribe();
            let pid_file = self.pid_file.clone();
            Box::pin(async move {
                let mut child = Command::new("/bin/sh")
                    .arg("-c")
                    .arg("sleep 30 & echo $! > \"$PID_FILE\"; wait")
                    .env("PID_FILE", pid_file)
                    .process_group(0)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .kill_on_drop(true)
                    .spawn()?;
                tokio::select! {
                    status = child.wait() => {
                        let _status = status?;
                    }
                    () = wait_for_signal(&mut cancel) => {
                        if let Some(pid) = child.id() {
                            let group = format!("-{pid}");
                            let _status = Command::new("/bin/kill")
                                .args(["-KILL", "--", &group])
                                .stdout(Stdio::null())
                                .stderr(Stdio::null())
                                .status()
                                .await?;
                        }
                        let _status = child.wait().await?;
                    }
                }
                Ok(())
            })
        }

        fn shutdown(&self) -> ActionFuture {
            let cancel = self.cancel.clone();
            Box::pin(async move {
                let _sent = cancel.send(true);
                Ok(())
            })
        }
    }

    #[cfg(unix)]
    async fn wait_for_signal(cancel: &mut watch::Receiver<bool>) {
        if *cancel.borrow() {
            return;
        }
        while cancel.changed().await.is_ok() {
            if *cancel.borrow() {
                return;
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cooperative_shutdown_reaps_descendant_process_group() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let pid_file = temporary.path().join("descendant.pid");
        let (cancel, _receiver) = watch::channel(false);
        let handler = Arc::new(ProcessHandler {
            cancel,
            pid_file: pid_file.clone(),
        });
        let (updates, _receiver) = mpsc::channel(1);
        let mut actions = JoinSet::new();
        spawn_action(
            &mut actions,
            Arc::clone(&handler),
            updates,
            TuiAction::ShowStatus,
        );
        let descendant = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(body) = fs::read_to_string(&pid_file)
                    && let Ok(pid) = body.trim().parse::<u32>()
                {
                    return pid;
                }
                tokio::task::yield_now().await;
            }
        })
        .await?;
        shutdown_actions(&handler, &mut actions).await?;
        let mut poll = tokio::time::interval(Duration::from_millis(10));
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                poll.tick().await;
                let status = Command::new("/bin/kill")
                    .args(["-0", &descendant.to_string()])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .await;
                if status.is_ok_and(|status| !status.success()) {
                    break;
                }
            }
        })
        .await?;
        Ok(())
    }
}
