use super::*;

#[tokio::test]
async fn named_workspace_is_reused_without_resetting_changes_and_original_stays_clean() -> Result<()>
{
    let (_temp, original, state) = super::super::tests::repo()?;
    let created = open_named_worktree(&original, &state, "feature-auth").await?;
    assert!(created.created);
    assert_eq!(
        created.path.file_name().and_then(|s| s.to_str()),
        Some("feature-auth")
    );
    let campaign = WorktreeCampaign::start(
        &created.path,
        &state,
        &["one".into(), "two".into()],
        Duration::from_secs(5),
        "named-dag",
    )
    .await?;
    let baseline = DagSettlementVerifier::capture(&created.path)?;
    for id in ["one", "two"] {
        let child = campaign.prepare_node(id, &baseline).await?;
        assert_ne!(child, created.path);
        campaign
            .mark_verified(id, &DagSettlementVerifier::capture(&child)?)
            .await?;
    }
    campaign.cleanup_fixture_nodes().await?;
    assert!(created.path.exists());
    drop(campaign);
    fs::write(created.path.join("a.rs"), "named change\n")?;
    let reopened = open_named_worktree(&original, &state, "feature-auth").await?;
    assert!(!reopened.created);
    assert_eq!(created.path, reopened.path);
    assert_eq!(
        fs::read_to_string(reopened.path.join("a.rs"))?,
        "named change\n"
    );
    assert_eq!(fs::read(original.join("a.rs"))?, b"old\n");
    clean(&original, Duration::from_secs(5)).await?;
    assert!(
        WorktreeCampaign::start(
            &created.path,
            &state,
            &["n".into()],
            Duration::from_secs(5),
            "must-reject-dirty"
        )
        .await
        .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn dirty_original_bad_name_and_foreign_named_directory_are_rejected() -> Result<()> {
    let (_temp, original, state) = super::super::tests::repo()?;
    assert!(
        open_named_worktree(&original, &state, "../escape")
            .await
            .is_err()
    );
    assert!(!state.exists());
    fs::write(original.join("untracked"), "dirty")?;
    assert!(
        open_named_worktree(&original, &state, "clean-required")
            .await
            .is_err()
    );
    assert!(!state.exists());
    fs::remove_file(original.join("untracked"))?;
    let n = open_named_worktree(&original, &state, "owned").await?;
    let old = n.path.with_extension("kept");
    fs::rename(&n.path, &old)?;
    fs::create_dir(&n.path)?;
    assert!(
        open_named_worktree(&original, &state, "owned")
            .await
            .is_err()
    );
    assert!(old.join("a.rs").exists());
    Ok(())
}
