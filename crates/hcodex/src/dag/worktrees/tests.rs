use super::*;
use tempfile::TempDir;
pub(super) fn repo() -> Result<(TempDir, PathBuf, PathBuf)> {
    let t = tempfile::tempdir()?;
    let r = t.path().join("repo");
    fs::create_dir(&r)?;
    let run = |args: &[&str]| -> Result<()> {
        let s = std::process::Command::new("git")
            .args(args)
            .current_dir(&r)
            .status()?;
        if !s.success() {
            return Err("fixture git failed".into());
        }
        Ok(())
    };
    run(&["init", "-q"])?;
    run(&["config", "user.name", "test"])?;
    run(&["config", "user.email", "test@example.invalid"])?;
    fs::write(r.join("a.rs"), b"old\n")?;
    run(&["add", "a.rs"])?;
    run(&["commit", "-qm", "fixture"])?;
    let state = t.path().join("state");
    Ok((t, r, state))
}
#[tokio::test]
async fn clean_precreation_overlay_and_exact_cleanup() -> Result<()> {
    let (_t, r, s) = repo()?;
    let ids = vec!["a".into(), "b".into()];
    let c = WorktreeCampaign::start(&r, &s, &ids, Duration::from_secs(10), "fixture").await?;
    assert!(
        WorktreeCampaign::start(&r, &s, &ids, Duration::from_secs(10), "fixture")
            .await
            .is_err()
    );
    let initial = DagSettlementVerifier::capture(&r)?;
    let a = c.prepare_node("a", &initial).await?;
    fs::write(r.join("new.rs"), b"verified parent\n")?;
    let next = DagSettlementVerifier::capture(&r)?;
    let b = c.prepare_node("b", &next).await?;
    assert_eq!(fs::read(b.join("new.rs"))?, b"verified parent\n");
    assert_ne!(a, b);
    assert!(c.cleanup_fixture_nodes().await.is_err());
    c.mark_verified("a", &DagSettlementVerifier::capture(&a)?)
        .await?;
    c.mark_verified("b", &DagSettlementVerifier::capture(&b)?)
        .await?;
    assert!(c.state_path().join("lifecycle.json").is_file());
    c.cleanup_fixture_nodes().await?;
    assert!(!a.exists() && !b.exists());
    assert_eq!(fs::read(r.join("new.rs"))?, b"verified parent\n");
    Ok(())
}
#[tokio::test]
async fn dirty_original_rejected_before_worktree_effects() -> Result<()> {
    for stage in ["untracked", "unstaged", "staged"] {
        let (_t, r, s) = repo()?;
        if stage == "untracked" {
            fs::write(r.join("extra"), b"x")?;
        } else {
            fs::write(r.join("a.rs"), b"changed")?;
            if stage == "staged" {
                git(&r, &["add", "a.rs"], Duration::from_secs(10)).await?;
            }
        }
        assert!(
            WorktreeCampaign::start(&r, &s, &["a".into()], Duration::from_secs(10), "fixture")
                .await
                .is_err()
        );
        assert!(!s.exists());
    }
    Ok(())
}
#[tokio::test]
async fn source_drift_and_hidden_node_files_retain_worktree() -> Result<()> {
    let (_t, r, s) = repo()?;
    let c =
        WorktreeCampaign::start(&r, &s, &["a".into()], Duration::from_secs(10), "fixture").await?;
    let baseline = DagSettlementVerifier::capture(&r)?;
    let a = c.prepare_node("a", &baseline).await?;
    fs::write(r.join("a.rs"), b"external")?;
    assert!(c.verify_original(&baseline).await.is_err());
    fs::create_dir(a.join(".hcodex"))?;
    fs::write(a.join(".hcodex/unique"), b"keep")?;
    assert!(
        c.mark_verified("a", &DagSettlementVerifier::capture(&a)?)
            .await
            .is_err()
    );
    assert!(c.cleanup_fixture_nodes().await.is_err());
    assert!(a.exists());
    Ok(())
}

#[tokio::test]
async fn replaced_root_refuses_cleanup_and_unrelated_worktree_survives() -> Result<()> {
    let (t, r, s) = repo()?;
    let other = t.path().join("user-worktree");
    git(
        &r,
        &[
            "worktree",
            "add",
            "--detach",
            other.to_str().ok_or("path")?,
            "HEAD",
        ],
        Duration::from_secs(10),
    )
    .await?;
    let c =
        WorktreeCampaign::start(&r, &s, &["a".into()], Duration::from_secs(10), "fixture").await?;
    let a = c
        .prepare_node("a", &DagSettlementVerifier::capture(&r)?)
        .await?;
    c.mark_verified("a", &DagSettlementVerifier::capture(&a)?)
        .await?;
    let retained = a.with_extension("retained");
    fs::rename(&a, &retained)?;
    fs::create_dir(&a)?;
    fs::copy(retained.join(".git"), a.join(".git"))?;
    fs::write(a.join("precious"), b"foreign")?;
    assert!(c.cleanup_fixture_nodes().await.is_err());
    assert_eq!(fs::read(a.join("precious"))?, b"foreign");
    assert!(other.join("a.rs").is_file());
    assert!(retained.exists());
    Ok(())
}
