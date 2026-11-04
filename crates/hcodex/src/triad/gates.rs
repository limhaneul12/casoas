use std::path::{Path, PathBuf};
use std::time::Duration;

use super::types::GateReceipt;

pub(crate) async fn run(workspace: &Path, limit: Duration) -> crate::Result<Vec<GateReceipt>> {
    let commands = commands_for(workspace)?;
    run_commands(workspace, limit, &commands).await
}

async fn run_commands(
    workspace: &Path,
    limit: Duration,
    commands: &[Vec<String>],
) -> crate::Result<Vec<GateReceipt>> {
    let mut receipts = Vec::with_capacity(commands.len());
    for command in commands {
        receipts.push(run_one(workspace, limit, command).await?);
    }
    Ok(receipts)
}

async fn run_one(
    workspace: &Path,
    limit: Duration,
    command: &[String],
) -> crate::Result<GateReceipt> {
    let label = command.join(" ");
    let (program, arguments) = command
        .split_first()
        .ok_or("verification command is empty")?;
    let arguments = arguments.iter().map(String::as_str).collect::<Vec<_>>();
    let proof = crate::verification::run(workspace, limit, program, &arguments).await?;
    let exit_code = proof.exit_code.unwrap_or(-1);
    Ok(GateReceipt {
        command: label,
        exit_code,
        passed: proof.outcome == crate::verification::GateOutcome::Completed && exit_code == 0,
        duration_seconds: proof.duration_seconds,
        stdout_digest: proof.stdout_digest,
        origin: "bridge_executed".to_owned(),
    })
}

fn commands_for(workspace: &Path) -> crate::Result<Vec<Vec<String>>> {
    let root = workspace.canonicalize()?;
    if root.join("Cargo.toml").is_file() {
        return Ok(vec![vec![
            "cargo".to_owned(),
            "xtask".to_owned(),
            "ci".to_owned(),
        ]]);
    }
    if root.join("Makefile").is_file() {
        return Ok(vec![vec!["make".to_owned(), "ci".to_owned()]]);
    }
    if root.join("pyproject.toml").is_file() {
        let has_tests = root.join("tests").is_dir()
            || std::fs::read_dir(&root)?
                .filter_map(std::result::Result::ok)
                .any(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.starts_with("test_") && name.ends_with(".py"))
                });
        let primary = if has_tests {
            vec!["uv", "run", "pytest", "-q", "-p", "no:cacheprovider"]
        } else {
            vec![
                "uv",
                "run",
                "python",
                "-c",
                "import ast; print('syntax_ok')",
            ]
        };
        let mut commands = vec![primary.into_iter().map(str::to_owned).collect()];
        commands.push(
            ["uv", "run", "ruff", "check", "."]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        );
        let harness = root.join(".agents/python_dev_harness/verify_mechanical_contracts.py");
        if harness.is_file() {
            commands.push(vec![
                "uv".to_owned(),
                "run".to_owned(),
                "python".to_owned(),
                ".agents/python_dev_harness/verify_mechanical_contracts.py".to_owned(),
            ]);
        }
        return Ok(commands);
    }
    Err("workspace has no allowlisted verification entrypoint".into())
}

pub(crate) fn resolve_workspace(path: PathBuf) -> crate::Result<PathBuf> {
    Ok(path.canonicalize()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn chooses_only_allowlisted_entrypoints() -> crate::Result<()> {
        let root = tempfile::tempdir()?;
        fs::write(root.path().join("Cargo.toml"), "[workspace]")?;
        assert_eq!(
            commands_for(root.path())?,
            vec![vec!["cargo", "xtask", "ci"]]
        );
        fs::remove_file(root.path().join("Cargo.toml"))?;
        fs::write(root.path().join("Makefile"), "ci:\n\t@true")?;
        assert_eq!(commands_for(root.path())?, vec![vec!["make", "ci"]]);
        fs::remove_file(root.path().join("Makefile"))?;
        fs::write(root.path().join("pyproject.toml"), "[tool.ruff]")?;
        fs::write(root.path().join("test_ok.py"), "def test_ok(): assert True")?;
        let python = commands_for(root.path())?;
        assert_eq!(python.len(), 2);
        assert!(python[0].iter().any(|part| part == "pytest"));
        assert!(python[1].iter().any(|part| part == "ruff"));
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn returns_one_bridge_receipt_per_executed_gate() -> crate::Result<()> {
        let root = tempfile::tempdir()?;
        let commands = vec![
            vec!["sh".to_owned(), "-c".to_owned(), "printf first".to_owned()],
            vec!["sh".to_owned(), "-c".to_owned(), "printf second".to_owned()],
        ];
        let receipts = run_commands(root.path(), Duration::from_secs(5), &commands).await?;
        assert_eq!(receipts.len(), 2);
        assert!(receipts.iter().all(|receipt| receipt.passed));
        assert!(
            receipts
                .iter()
                .all(|receipt| receipt.origin == "bridge_executed")
        );
        assert_ne!(receipts[0].stdout_digest, receipts[1].stdout_digest);
        Ok(())
    }
}
