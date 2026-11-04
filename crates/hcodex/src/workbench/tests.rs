use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::*;
use crate::storage::NewJob;
use crate::triad::{adjudicate, canonical_digest, inspect_role};

fn request(workspace: &Path, id: &str, task: &str, model: &str) -> NewJob {
    NewJob {
        task_type: task.into(),
        workspace_dir: workspace.into(),
        model: Some(model.into()),
        metadata: BTreeMap::new(),
        job_id: Some(id.into()),
        idempotency_key: None,
        provider: Some("fixture".into()),
        engine: Some("fixture".into()),
        created_at: None,
    }
}

async fn seed_council(
    gateway: &Gateway,
    workspace: &Path,
    verdicts: [&str; 3],
    file: Option<&Path>,
) -> crate::Result<String> {
    let mut snapshots = BTreeMap::new();
    for (role, verdict) in ["argon", "sonnet", "codex"].into_iter().zip(verdicts) {
        let id = format!("cdx-role-{role}");
        let model = format!("model-{role}");
        let artifact = json!({"verdict":verdict,"summary":format!("{role} reviewed")});
        let receipt = json!({"status":"succeeded","requested_model":model,"actual_model":model,"model_mismatch":false});
        let saved_artifact = artifact.clone();
        let saved_receipt = receipt.clone();
        let saved_id = id.clone();
        let job = request(workspace, &id, &format!("{role}_review"), &model);
        gateway
            .database(move |store| {
                store.create_job(job)?;
                store.complete_job_with_details(
                    &saved_id,
                    json!({"structured_output":saved_artifact}),
                    None,
                    Some(saved_receipt),
                    None,
                    BTreeMap::new(),
                )?;
                Ok(())
            })
            .await?;
        snapshots.insert(
            role.to_owned(),
            inspect_role(
                role,
                Some(json!({
                    "ok":true,"status":"succeeded","job_id":id,
                    "result":{"receipt":receipt},"structured_output":artifact,
                })),
            ),
        );
    }
    let compact = snapshots
        .iter()
        .map(|(role, value)| (role.clone(), value.compact()))
        .collect::<BTreeMap<_, _>>();
    let mut digests = serde_json::Map::new();
    if let Some(file) = file {
        let bytes = std::fs::read(workspace.join(file))?;
        digests.insert(
            file.to_string_lossy().into_owned(),
            Value::String(format!("sha256:{:x}", Sha256::digest(bytes))),
        );
    }
    let mut aggregate = json!({
        "schema_version":2,"task_type":"triad_orchestration","collaboration_id":"cdx-council",
        "orchestration_mode":"council","source_task_digest":"sha256:task","workspace_dir":workspace,
        "evidence_scope":{"workspace_dir":workspace,"files_inspected_digests":digests,
            "unverified_or_missing_files":[],"scope_digest":"sha256:scope"},
        "roles":compact,"role_evidence_dimensions":{},"adjudication":adjudicate(&snapshots),
    });
    aggregate["snapshot_digest"] = Value::String(canonical_digest(&aggregate)?);
    let job = request(workspace, "cdx-council", "triad_orchestration", "tri-polar");
    gateway
        .database(move |store| {
            store.create_job(job)?;
            store.complete_job("cdx-council", aggregate, None, None)?;
            Ok(())
        })
        .await?;
    Ok("cdx-council".into())
}

#[tokio::test]
async fn board_survives_restart_and_exposes_compact_council() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let gateway = Gateway::new(root.path().into());
    seed_council(&gateway, workspace.path(), ["PASS", "PASS", "PASS"], None).await?;
    drop(gateway);
    let result = board(&Gateway::new(root.path().into()), workspace.path(), 10).await?;
    assert_eq!(result["count"], 1);
    assert_eq!(result["items"][0]["detail"]["kind"], "review_council");
    Ok(())
}

#[tokio::test]
async fn board_projects_handoff_without_raw_provider_output() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let gateway = Gateway::new(root.path().into());
    let mut job = request(
        workspace.path(),
        "ckp-campaign-node",
        "dag_checkpoint",
        "dag-node",
    );
    job.metadata.insert("checkpoint_kind".into(), "node".into());
    job.metadata.insert("node_id".into(), "node".into());
    gateway
        .database(move |store| {
            store.create_job(job)?;
            store.complete_job(
                "ckp-campaign-node",
                json!({
                    "provider_output":"must not escape",
                    "handoff":{
                        "content_digest":"sha256:handoff",
                        "agent_report":{"summary":"bounded summary","decisions":["decision"],"caveats":[]},
                        "parents":[]
                    }
                }),
                None,
                None,
            )?;
            Ok(())
        })
        .await?;
    let result = board(&gateway, workspace.path(), 10).await?;
    assert_eq!(result["items"][0]["detail"]["handoff_status"], "available");
    assert_eq!(
        result["items"][0]["detail"]["handoff"]["summary"],
        "bounded summary"
    );
    assert!(!serde_json::to_string(&result)?.contains("must not escape"));
    Ok(())
}

#[tokio::test]
async fn votes_require_current_source_and_unanimous_pass() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    std::fs::write(workspace.path().join("scope.rs"), "fn scope() {}")?;
    let gateway = Gateway::new(root.path().into());
    let id = seed_council(
        &gateway,
        workspace.path(),
        ["PASS", "PASS", "PASS"],
        Some(Path::new("scope.rs")),
    )
    .await?;
    let current = votes(&gateway, workspace.path(), &id).await?;
    assert_eq!(current["source_status"]["status"], "current_scoped");
    assert_eq!(current["current_review_approved"], true);
    std::fs::write(workspace.path().join("scope.rs"), "fn changed() {}")?;
    let stale = votes(&gateway, workspace.path(), &id).await?;
    assert_eq!(stale["source_status"]["status"], "stale");
    assert_eq!(stale["current_review_approved"], false);
    Ok(())
}

#[tokio::test]
async fn warn_block_empty_scope_and_foreign_workspace_are_not_approved() -> crate::Result<()> {
    for verdicts in [["WARN", "WARN", "WARN"], ["PASS", "PASS", "BLOCK"]] {
        let root = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let other = tempfile::tempdir()?;
        let gateway = Gateway::new(root.path().into());
        let id = seed_council(&gateway, workspace.path(), verdicts, None).await?;
        let result = votes(&gateway, workspace.path(), &id).await?;
        assert_eq!(result["source_status"]["status"], "unknown");
        assert_eq!(result["current_review_approved"], false);
        assert!(votes(&gateway, other.path(), &id).await.is_err());
    }
    Ok(())
}

#[test]
fn council_request_is_review_not_verification_claim() {
    let value = council_request("review design", Path::new("/tmp/workspace"));
    assert!(crate::catalog::arguments("heterarchy_triad_orchestrate", value.clone()).is_ok());
    assert_eq!(value["mode"], "council");
    assert_eq!(value["run_verification_gates"], false);
    assert_eq!(value["async_background"], false);
    assert!(value.get("plan_only").is_none());
}

#[tokio::test]
async fn missing_ballots_keep_their_public_shape_and_cannot_approve() -> crate::Result<()> {
    let state = tempfile::tempdir()?;
    let workspace = tempfile::tempdir()?;
    let gateway = Gateway::new(state.path().into());
    let snapshots = ["argon", "sonnet", "codex"]
        .into_iter()
        .map(|role| (role.to_owned(), inspect_role(role, None)))
        .collect::<BTreeMap<_, _>>();
    let mut source = json!({
        "schema_version":2,"task_type":"triad_orchestration",
        "collaboration_id":"cdx-missing-ballots","workspace_dir":workspace.path(),
        "source_task_digest":"sha256:task",
        "roles":snapshots,"adjudication":adjudicate(&snapshots),
        "evidence_scope":{"workspace_dir":workspace.path(),"files_inspected_digests":{},"unverified_or_missing_files":[]}
    });
    source["snapshot_digest"] = Value::String(canonical_digest(&source)?);
    let job = request(
        workspace.path(),
        "cdx-missing-ballots",
        "triad_orchestration",
        "tri-polar",
    );
    gateway
        .database(move |store| {
            store.create_job(job)?;
            store.complete_job("cdx-missing-ballots", source, None, None)?;
            Ok(())
        })
        .await?;
    let result = votes(&gateway, workspace.path(), "cdx-missing-ballots").await?;
    assert_eq!(
        result["ballots"],
        json!([
            {"role":"argon","valid":false,"reason":"missing_role_job","verdict":null},
            {"role":"codex","valid":false,"reason":"missing_role_job","verdict":null},
            {"role":"sonnet","valid":false,"reason":"missing_role_job","verdict":null}
        ])
    );
    assert_eq!(
        result["adjudication"]["consensus_status"],
        "incomplete_no_consensus"
    );
    assert_eq!(result["current_review_approved"], false);
    assert_eq!(result["historical_review_approved"], false);
    Ok(())
}

#[tokio::test]
async fn ballot_receipt_precedence_and_failure_shapes_are_preserved() -> crate::Result<()> {
    let valid = json!({"requested_model":"model-argon","actual_model":"model-argon",
        "status":"succeeded","model_mismatch":false});
    let cases = [
        json!({"missing_job":true}),
        json!({"result":{"receipt":valid},"valid":true}),
        json!({"result":{"result":{"receipt":valid}},"valid":true}),
        // Persisted Option<Value> null is read back as None, so fallback applies.
        json!({"preferred":null,"result":{"receipt":valid},"valid":true}),
        json!({"preferred":{},"result":{"receipt":valid}}),
        json!({"result":{"receipt":null,"result":{"receipt":valid}}}),
        json!({"preferred":{"requested_model":"model-argon","actual_model":"model-argon","status":"completed","model_mismatch":false},"valid":true}),
        json!({"preferred":{"requested_model":"model-argon","actual_model":"model-argon","status":"succeeded"}}),
        json!({"preferred":{"requested_model":"model-argon","actual_model":"model-argon","status":"succeeded","model_mismatch":"false"}}),
        json!({"preferred":{"requested_model":"other","actual_model":"model-argon","status":"succeeded","model_mismatch":false}}),
        json!({"preferred":valid,"job_model":"other"}),
        json!({"preferred":valid,"foreign":true}),
        json!({"preferred":valid,"failed":true}),
    ];
    for case in cases {
        let state = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let foreign = tempfile::tempdir()?;
        let gateway = Gateway::new(state.path().into());
        let result = case.get("result").cloned().unwrap_or_else(|| json!({}));
        let preferred = case.get("preferred").cloned();
        if case["missing_job"] != true {
            let job_workspace = if case["foreign"] == true {
                foreign.path()
            } else {
                workspace.path()
            };
            let model = case["job_model"].as_str().unwrap_or("model-argon");
            let job = request(job_workspace, "cdx-receipt-role", "fixture_review", model);
            let failed = case["failed"] == true;
            gateway
                .database(move |store| {
                    store.create_job(job)?;
                    if failed {
                        store.fail_job_with_result(
                            "cdx-receipt-role",
                            "fixture failure".into(),
                            Some(result),
                            preferred,
                        )?;
                    } else {
                        store.complete_job_with_details(
                            "cdx-receipt-role",
                            result,
                            None,
                            preferred,
                            None,
                            BTreeMap::new(),
                        )?;
                    }
                    Ok(())
                })
                .await?;
        }
        let mut snapshots = ["argon", "sonnet", "codex"]
            .into_iter()
            .map(|role| (role.to_owned(), inspect_role(role, None)))
            .collect::<BTreeMap<_, _>>();
        let role = snapshots.get_mut("argon").ok_or("argon fixture")?;
        role.job_id = Some("cdx-receipt-role".into());
        role.model_id = Some("model-argon".into());
        let mut source = json!({"schema_version":2,"task_type":"triad_orchestration",
            "collaboration_id":"cdx-receipt-council","workspace_dir":workspace.path(),
            "source_task_digest":"sha256:task","roles":snapshots,"adjudication":adjudicate(&snapshots),
            "evidence_scope":{"workspace_dir":workspace.path(),"files_inspected_digests":{}}});
        source["snapshot_digest"] = Value::String(canonical_digest(&source)?);
        let job = request(
            workspace.path(),
            "cdx-receipt-council",
            "triad_orchestration",
            "tri-polar",
        );
        gateway
            .database(move |store| {
                store.create_job(job)?;
                store.complete_job("cdx-receipt-council", source, None, None)?;
                Ok(())
            })
            .await?;
        let actual = votes(&gateway, workspace.path(), "cdx-receipt-council").await?;
        let expected = if case["missing_job"] == true {
            json!({"role":"argon","job_id":"cdx-receipt-role","valid":false,"reason":"role_job_missing","verdict":null})
        } else {
            let chosen = case
                .get("preferred")
                .filter(|value| !value.is_null())
                .or_else(|| case.pointer("/result/receipt"))
                .or_else(|| case.pointer("/result/result/receipt"));
            json!({"role":"argon","job_id":"cdx-receipt-role","verdict":null,
                "requested_model":chosen.and_then(|v|v["requested_model"].as_str()),
                "actual_model":chosen.and_then(|v|v["actual_model"].as_str()),
                "valid":case["valid"] == true,
                "reason":if case["valid"] == true {"verified_role_receipt"}else{"invalid_role_receipt"}})
        };
        assert_eq!(actual["ballots"][0], expected, "{case}");
        assert_eq!(actual["duplicate_role_job"], false);
        assert_eq!(actual["current_review_approved"], false);
    }
    Ok(())
}

#[test]
fn scoped_council_request_is_catalog_valid_and_tui_parser_is_strict() -> crate::Result<()> {
    let workspace = tempfile::tempdir()?;
    std::fs::create_dir(workspace.path().join("src"))?;
    std::fs::write(workspace.path().join("src/lib.rs"), "lib")?;
    std::fs::write(workspace.path().join("Cargo.toml"), "manifest")?;
    let value = council_request_with_files(
        "review scoped source",
        workspace.path(),
        &[
            "src/lib.rs".into(),
            "src/lib.rs".into(),
            "Cargo.toml".into(),
        ],
    )?;
    assert!(crate::catalog::arguments("heterarchy_triad_orchestrate", value.clone()).is_ok());
    assert_eq!(
        value["files_inspected"],
        json!(["Cargo.toml", "src/lib.rs"])
    );
    let parsed = parse_council_request(
        "/council --file src/lib.rs --file Cargo.toml -- review this",
        workspace.path(),
    )?;
    assert_eq!(parsed["files_inspected"], value["files_inspected"]);
    assert!(
        parse_council_request(
            "/council --file path with spaces -- review",
            workspace.path(),
        )
        .is_err()
    );
    assert!(council_request_with_files("bad", workspace.path(), &["../escape.rs".into()]).is_err());
    Ok(())
}

#[test]
fn freshness_rejects_unsafe_sources_and_marks_deleted_source_stale() -> crate::Result<()> {
    let workspace = tempfile::tempdir()?;
    std::fs::write(workspace.path().join("source.rs"), "source")?;
    let digest = format!("sha256:{:x}", Sha256::digest(b"source"));
    assert!(
        source_freshness(
            workspace.path(),
        &json!({"workspace_dir":workspace.path(),"files_inspected_digests":{"../escape.rs":digest}})
        )
        .is_err()
    );
    assert_eq!(
        source_freshness(
            workspace.path(),
            &json!({"workspace_dir":workspace.path(),"files_inspected_digests":{"source.rs":"sha256:bad"}})
        )?["status"],
        "unknown"
    );
    std::fs::remove_file(workspace.path().join("source.rs"))?;
    assert_eq!(
        source_freshness(
            workspace.path(),
            &json!({"workspace_dir":workspace.path(),"files_inspected_digests":{"source.rs":digest}})
        )?["status"],
        "stale"
    );
    assert!(has_duplicate_ids(["same", "same"]));
    Ok(())
}

#[cfg(unix)]
#[test]
fn freshness_rejects_symlinked_source_path() -> crate::Result<()> {
    use std::os::unix::fs::symlink;

    let workspace = tempfile::tempdir()?;
    let outside = tempfile::NamedTempFile::new()?;
    symlink(outside.path(), workspace.path().join("source.rs"))?;
    let digest = format!("sha256:{:x}", Sha256::digest(b"outside"));
    assert!(
        source_freshness(
            workspace.path(),
        &json!({"workspace_dir":workspace.path(),"files_inspected_digests":{"source.rs":digest}})
        )
        .is_err()
    );
    Ok(())
}
