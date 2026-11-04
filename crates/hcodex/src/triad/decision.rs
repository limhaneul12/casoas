use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::gateway::{Gateway, optional, text};
use crate::storage::{HandoffStatus, JobStatus};

use super::adjudication::{adjudicate, canonical_digest};
use super::orchestrator::snapshot_valid;
use super::types::RoleSnapshot;

pub async fn decision_pack(gateway: &Gateway, args: &Value) -> crate::Result<Value> {
    let expected_workspace = optional(args, "workspace_dir")
        .map(PathBuf::from)
        .map(|path| path.canonicalize())
        .transpose()?;
    let collaboration_id = optional(args, "collaboration_id");
    let expected_scope = optional(args, "expected_scope_digest");
    let source = match (
        collaboration_id.as_deref(),
        expected_workspace.as_deref(),
        expected_scope.as_deref(),
    ) {
        (Some(identifier), Some(expected_workspace), Some(expected_scope)) => {
            load_source(
                gateway,
                identifier,
                Some(expected_workspace),
                Some(expected_scope),
            )
            .await?
        }
        _ => None,
    };
    let adjudication = source.as_ref().and_then(|value| recompute(value).ok());
    let verified = source.is_some() && adjudication.is_some();
    let collaboration_status = adjudication
        .as_ref()
        .map_or("unverified", |value| value.collaboration_status.as_str());
    let consensus_status = adjudication
        .as_ref()
        .map_or("unverified", |value| value.consensus_status.as_str());
    let consensus_reached = adjudication
        .as_ref()
        .is_some_and(|value| value.consensus_reached);
    let review_approved = adjudication
        .as_ref()
        .is_some_and(|value| value.review_approved);
    let resolution_verified = if let Some(source) = source.as_ref() {
        verify_resolution(gateway, source, args).await?
    } else {
        false
    };
    let source_digest = source
        .as_ref()
        .and_then(|value| value["snapshot_digest"].as_str());
    let source_collaboration_id = source
        .as_ref()
        .and_then(|value| value["collaboration_id"].as_str());
    let source_task_digest = source
        .as_ref()
        .and_then(|value| value["source_task_digest"].as_str());
    let source_workspace = source
        .as_ref()
        .and_then(|value| value["workspace_dir"].as_str());
    let source_scope = source.as_ref().map(|value| value["evidence_scope"].clone());
    let role_dimensions = source
        .as_ref()
        .map(|value| value["role_evidence_dimensions"].clone());
    let decision_input = json!({
        "project_scope": text(args,"project_scope"),
        "topic_title": text(args,"topic_title"),
        "argon_architecture_intent": text(args,"argon_architecture_intent"),
        "sonnet_craft_or_review_summary": text(args,"sonnet_craft_or_review_summary"),
        "codex_audit_or_synthesis_summary": text(args,"codex_audit_or_synthesis_summary"),
        "final_resolution": text(args,"final_resolution"),
        "verification_evidence": text(args,"verification_evidence"),
        "collaboration_id": collaboration_id,
        "source_snapshot_digest": source_digest,
        "collaboration_status": collaboration_status,
        "consensus_status": consensus_status,
        "review_approved": review_approved,
        "resolution_verified": resolution_verified,
    });
    let digest = canonical_digest(&decision_input)?;
    let digest_hex = digest.trim_start_matches("sha256:");
    let digest_short = &digest_hex[..digest_hex.len().min(16)];
    let logical_identity = format!("{}/decisions/{}", text(args, "project_scope"), digest_short);
    let markdown = render_markdown(
        args,
        consensus_status,
        collaboration_status,
        source_digest,
        resolution_verified,
    );
    Ok(json!({
        "ok": true,
        "project_scope":text(args,"project_scope"),
        "title": if consensus_reached {"Heterarchy Consensus"} else {"Heterarchy Decision"},
        "digest": digest,
        "source_verified": verified,
        "evidence_status":if verified {"verified"} else {"unverified"},
        "requested_collaboration_id":collaboration_id,
        "collaboration_id":source_collaboration_id,
        "source_task_digest":source_task_digest,
        "source_snapshot_digest": source_digest,
        "workspace_dir":source_workspace,
        "expected_workspace_dir": expected_workspace,
        "expected_scope_digest": expected_scope,
        "evidence_scope":source_scope,
        "role_evidence_dimensions":role_dimensions,
        "collaboration_status": collaboration_status,
        "consensus_status": consensus_status,
        "consensus_reached": consensus_reached,
        "review_approved": review_approved,
        "resolution_verified": resolution_verified,
        "idempotency_key": format!("heterarchy-argon-codex-{digest_short}"),
        "logical_identity": logical_identity,
        "markdown_content": markdown,
        "recommended_next_step": if consensus_reached {"Store through Alexandria verified upsert and read back the canonical note."} else {"Resolve incomplete or conflicting role evidence before treating this as consensus."},
        "adjudication": adjudication,
    }))
}

async fn load_source(
    gateway: &Gateway,
    collaboration_id: &str,
    expected_workspace: Option<&Path>,
    expected_scope: Option<&str>,
) -> crate::Result<Option<Value>> {
    if !collaboration_id.starts_with("cdx-") {
        return Ok(None);
    }
    let identifier = collaboration_id.to_owned();
    let record = gateway
        .database(move |store| store.get_job(&identifier))
        .await?;
    let Some(record) = record else {
        return Ok(None);
    };
    if record.task_type != "triad_orchestration" || record.status != JobStatus::Completed {
        return Ok(None);
    }
    if let Some(expected) = expected_workspace
        && PathBuf::from(record.workspace_dir).canonicalize()? != expected
    {
        return Ok(None);
    }
    let Some(result) = record.result else {
        return Ok(None);
    };
    if !snapshot_valid(&result)? {
        return Ok(None);
    }
    if let Some(scope) = expected_scope
        && result["evidence_scope"]["scope_digest"].as_str() != Some(scope)
    {
        return Ok(None);
    }
    let Some(role_values) = result["roles"].as_object() else {
        return Ok(None);
    };
    let mut role_fences = Vec::new();
    let mut handoff_fences = Vec::new();
    for value in role_values.values() {
        let Ok(snapshot) = serde_json::from_value::<RoleSnapshot>(value.clone()) else {
            return Ok(None);
        };
        if !snapshot.review_artifact_present {
            continue;
        }
        let Some(expected_digest) = snapshot.review_artifact_digest else {
            return Ok(None);
        };
        if let Some(job_id) = snapshot.job_id {
            role_fences.push((job_id, expected_digest));
        } else if let Some(ticket_id) = snapshot.handoff_ticket_id {
            handoff_fences.push((ticket_id, snapshot.verdict, expected_digest));
        } else {
            return Ok(None);
        }
    }
    let role_jobs_valid = gateway
        .database(move |store| {
            for (job_id, expected_digest) in role_fences {
                let Some(job) = store.get_job(&job_id)? else {
                    return Ok(false);
                };
                if job.status != JobStatus::Completed {
                    return Ok(false);
                }
                let Some(result) = job.result else {
                    return Ok(false);
                };
                let Some(artifact) = result
                    .get("structured_output")
                    .filter(|value| value.is_object())
                else {
                    return Ok(false);
                };
                if canonical_digest(artifact)? != expected_digest {
                    return Ok(false);
                }
            }
            for (ticket_id, expected_verdict, expected_digest) in handoff_fences {
                let Some(ticket) = store.get_handoff(&ticket_id)? else {
                    return Ok(false);
                };
                let actual = ticket.verdict().map(serde_json::to_value).transpose()?;
                let expected = expected_verdict.map(serde_json::to_value).transpose()?;
                let artifact = json!({
                    "verdict":ticket.verdict(),
                    "summary":ticket.resolution_note.clone().unwrap_or_else(||"Argon handoff completed.".to_owned()),
                    "alexandria_ref":ticket.alexandria_ref,
                });
                if ticket.status != HandoffStatus::Completed
                    || actual != expected
                    || canonical_digest(&artifact)? != expected_digest
                {
                    return Ok(false);
                }
            }
            Ok(true)
        })
        .await?;
    Ok(role_jobs_valid.then_some(result))
}

fn recompute(source: &Value) -> crate::Result<super::types::Adjudication> {
    let object = source["roles"]
        .as_object()
        .ok_or("triad source has no role snapshots")?;
    if object.len() != 3 {
        return Err("triad source role set is incomplete".into());
    }
    let roles = object
        .iter()
        .map(|(role, value)| {
            Ok((
                role.clone(),
                serde_json::from_value::<RoleSnapshot>(value.clone())?,
            ))
        })
        .collect::<crate::Result<std::collections::BTreeMap<_, _>>>()?;
    for snapshot in roles.values() {
        match (&snapshot.artifact, &snapshot.review_artifact_digest) {
            (Some(artifact), Some(digest)) if canonical_digest(artifact)? == *digest => {}
            (None, Some(_)) | (None, None) => {}
            _ => return Err("role artifact digest mismatch".into()),
        }
    }
    let recomputed = adjudicate(&roles);
    if source["adjudication"] != serde_json::to_value(&recomputed)? {
        return Err("saved adjudication does not match role evidence".into());
    }
    Ok(recomputed)
}

async fn verify_resolution(gateway: &Gateway, source: &Value, args: &Value) -> crate::Result<bool> {
    let Some(roles) = source["roles"].as_object() else {
        return Ok(false);
    };
    let mut fences = Vec::new();
    for role in ["argon", "sonnet", "codex"] {
        let Some(snapshot_value) = roles.get(role) else {
            return Ok(false);
        };
        let snapshot: RoleSnapshot = serde_json::from_value(snapshot_value.clone())?;
        let (Some(job_id), Some(digest)) = (snapshot.job_id, snapshot.review_artifact_digest)
        else {
            return Ok(false);
        };
        fences.push((role.to_owned(), job_id, digest));
    }
    let argon_intent = text(args, "argon_architecture_intent").trim().to_owned();
    let final_resolution = text(args, "final_resolution").trim().to_owned();
    let sonnet_summary = text(args, "sonnet_craft_or_review_summary")
        .trim()
        .to_owned();
    let codex_summary = text(args, "codex_audit_or_synthesis_summary")
        .trim()
        .to_owned();
    gateway
        .database(move |store| {
            for (role, job_id, expected_digest) in fences {
                let Some(job) = store.get_job(&job_id)? else {
                    return Ok(false);
                };
                let Some(result) = job.result else {
                    return Ok(false);
                };
                let Some(artifact) = result
                    .get("structured_output")
                    .or_else(|| result.get("data"))
                    .filter(|value| value.is_object())
                else {
                    return Ok(false);
                };
                if job.status != JobStatus::Completed
                    || canonical_digest(artifact)? != expected_digest
                {
                    return Ok(false);
                }
                let summary = artifact
                    .get("summary")
                    .or_else(|| artifact.get("executive_critique"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim();
                let matches = match role.as_str() {
                    "argon" => {
                        let recommendation = artifact
                            .get("final_recommendation")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .trim();
                        [summary, recommendation].contains(&argon_intent.as_str())
                            && [summary, recommendation].contains(&final_resolution.as_str())
                    }
                    "sonnet" => sonnet_summary == summary,
                    "codex" => {
                        let note = artifact
                            .get("alexandria_decision_note")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .trim();
                        [summary, note].contains(&codex_summary.as_str())
                    }
                    _ => false,
                };
                if summary.is_empty() || !matches {
                    return Ok(false);
                }
            }
            Ok(true)
        })
        .await
}

fn render_markdown(
    args: &Value,
    consensus: &str,
    collaboration: &str,
    source_digest: Option<&str>,
    resolution_verified: bool,
) -> String {
    let title_prefix = if consensus == "consensus_verified" {
        "Heterarchy Consensus"
    } else {
        "Heterarchy Decision"
    };
    format!(
        "# [{title_prefix}] {}\n\n- Project: `{}`\n- Source collaboration ID: `{}`\n- Collaboration: `{collaboration}`\n- Consensus: `{consensus}`\n- Source snapshot: `{}`\n- Resolution verified: `{resolution_verified}`\n\n## Argon architecture intent\n\n{}\n\n## Claude craft or review\n\n{}\n\n## Codex audit or synthesis\n\n{}\n\n## Resolution\n\n{}\n\n## Verification evidence\n\n{}\n",
        text(args, "topic_title"),
        text(args, "project_scope"),
        optional(args, "collaboration_id").unwrap_or_else(|| "unverified".to_owned()),
        source_digest.unwrap_or("unverified"),
        text(args, "argon_architecture_intent"),
        text(args, "sonnet_craft_or_review_summary"),
        text(args, "codex_audit_or_synthesis_summary"),
        text(args, "final_resolution"),
        text(args, "verification_evidence"),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::storage::NewJob;
    use crate::triad::adjudication::{adjudicate, canonical_digest, inspect_role};

    #[test]
    fn decision_markdown_is_deterministic() {
        let args = json!({
            "topic_title":"Cutover", "project_scope":"hcodex", "argon_architecture_intent":"A",
            "sonnet_craft_or_review_summary":"S", "codex_audit_or_synthesis_summary":"C",
            "final_resolution":"R", "verification_evidence":"V"
        });
        assert_eq!(
            render_markdown(
                &args,
                "consensus_verified",
                "complete",
                Some("sha256:abc"),
                false,
            ),
            render_markdown(
                &args,
                "consensus_verified",
                "complete",
                Some("sha256:abc"),
                false,
            ),
        );
    }

    #[tokio::test]
    async fn unverified_text_never_claims_consensus() -> crate::Result<()> {
        let state = tempfile::tempdir()?;
        let gateway = Gateway::new(state.path().to_path_buf());
        let args = json!({
            "project_scope":"hcodex", "topic_title":"Cutover",
            "argon_architecture_intent":"intent", "sonnet_craft_or_review_summary":"craft",
            "codex_audit_or_synthesis_summary":"audit", "final_resolution":"ship",
            "verification_evidence":"none", "collaboration_id":null,
            "workspace_dir":null, "expected_scope_digest":null
        });
        let result = decision_pack(&gateway, &args).await?;
        assert_eq!(result["title"], "Heterarchy Decision");
        assert_eq!(result["source_verified"], false);
        assert_eq!(result["consensus_reached"], false);
        assert_eq!(result["resolution_verified"], false);
        assert!(
            result["markdown_content"]
                .as_str()
                .is_some_and(|markdown| markdown.starts_with("# [Heterarchy Decision]"))
        );
        Ok(())
    }

    #[tokio::test]
    async fn source_fences_reject_wrong_workspace_scope_and_tampering() -> crate::Result<()> {
        let state = tempfile::tempdir()?;
        let workspace = tempfile::tempdir()?;
        let other_workspace = tempfile::tempdir()?;
        let gateway = Gateway::new(state.path().to_path_buf());
        let (collaboration_id, scope_digest) = seed_snapshot(&gateway, workspace.path()).await?;
        let valid_args = decision_args(&collaboration_id, workspace.path(), &scope_digest);
        assert_eq!(
            decision_pack(&gateway, &valid_args).await?["source_verified"],
            true
        );
        assert_eq!(
            decision_pack(&gateway, &valid_args).await?["resolution_verified"],
            true
        );
        let valid_pack = decision_pack(&gateway, &valid_args).await?;
        assert!(
            valid_pack["markdown_content"]
                .as_str()
                .is_some_and(|markdown| markdown.starts_with("# [Heterarchy Consensus]"))
        );
        assert!(
            valid_pack["markdown_content"]
                .as_str()
                .is_some_and(|markdown| markdown.contains(&collaboration_id))
        );

        let wrong_workspace =
            decision_args(&collaboration_id, other_workspace.path(), &scope_digest);
        assert_eq!(
            decision_pack(&gateway, &wrong_workspace).await?["source_verified"],
            false
        );
        let wrong_scope = decision_args(
            &collaboration_id,
            workspace.path(),
            "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        );
        assert_eq!(
            decision_pack(&gateway, &wrong_scope).await?["source_verified"],
            false
        );
        let mut missing_scope = valid_args.clone();
        missing_scope["expected_scope_digest"] = Value::Null;
        assert_eq!(
            decision_pack(&gateway, &missing_scope).await?["source_verified"],
            false
        );

        let connection = rusqlite::Connection::open(state.path().join("jobs/jobs.db"))?;
        let raw: String = connection.query_row(
            "SELECT record_json FROM jobs WHERE job_id = ?",
            [&collaboration_id],
            |row| row.get(0),
        )?;
        let mut record: Value = serde_json::from_str(&raw)?;
        record["result"]["workspace_dir"] = Value::String("tampered".to_owned());
        connection.execute(
            "UPDATE jobs SET record_json = ? WHERE job_id = ?",
            [serde_json::to_string(&record)?, collaboration_id.clone()],
        )?;
        assert_eq!(
            decision_pack(&gateway, &valid_args).await?["source_verified"],
            false
        );
        Ok(())
    }

    async fn seed_snapshot(gateway: &Gateway, workspace: &Path) -> crate::Result<(String, String)> {
        let workspace = workspace.canonicalize()?;
        let mut snapshots = BTreeMap::new();
        for role in ["argon", "sonnet", "codex"] {
            let job_id = format!("cdx-{role}role");
            let artifact = match role {
                "argon" => json!({
                    "verdict":"PASS","summary":"Argon architectural intent approved.",
                    "final_recommendation":"Adopt the CAS-guarded implementation."
                }),
                "sonnet" => json!({"verdict":"PASS","summary":"Sonnet surgical craft summary."}),
                _ => json!({"verdict":"PASS","executive_critique":"Codex audit verified CAS fix."}),
            };
            let request = NewJob {
                task_type: format!("{role}_review"),
                workspace_dir: workspace.clone(),
                model: Some(role.to_owned()),
                metadata: BTreeMap::new(),
                job_id: Some(job_id.clone()),
                idempotency_key: None,
                provider: Some(role.to_owned()),
                engine: Some("fixture".to_owned()),
                created_at: None,
            };
            let saved_artifact = artifact.clone();
            let saved_job_id = job_id.clone();
            gateway
                .database(move |store| {
                    store.create_job(request)?;
                    store.complete_job(
                        &saved_job_id,
                        json!({"structured_output":saved_artifact}),
                        None,
                        None,
                    )?;
                    Ok(())
                })
                .await?;
            snapshots.insert(
                role.to_owned(),
                inspect_role(
                    role,
                    Some(json!({
                        "ok":true,"status":"succeeded","job_id":job_id,"structured_output":artifact
                    })),
                ),
            );
        }
        let adjudication = adjudicate(&snapshots);
        let compact_snapshots = snapshots
            .iter()
            .map(|(role, snapshot)| (role.clone(), snapshot.compact()))
            .collect::<BTreeMap<_, _>>();
        let scope_digest =
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let mut aggregate = json!({
            "schema_version":2,"collaboration_id":"cdx-aggregate1","workspace_dir":workspace,
            "source_task_digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "roles":compact_snapshots,"adjudication":adjudication,
            "evidence_scope":{"workspace_dir":workspace,"files_inspected_digests":{},"unverified_or_missing_files":[],"scope_digest":scope_digest}
        });
        aggregate["snapshot_digest"] = Value::String(canonical_digest(&aggregate)?);
        let request = NewJob {
            task_type: "triad_orchestration".to_owned(),
            workspace_dir: workspace.clone(),
            model: Some("tri-polar".to_owned()),
            metadata: BTreeMap::new(),
            job_id: Some("cdx-aggregate1".to_owned()),
            idempotency_key: None,
            provider: Some("heterarchy".to_owned()),
            engine: Some("triad".to_owned()),
            created_at: None,
        };
        gateway
            .database(move |store| {
                store.create_job(request)?;
                store.complete_job("cdx-aggregate1", aggregate, None, None)?;
                Ok(())
            })
            .await?;
        Ok(("cdx-aggregate1".to_owned(), scope_digest.to_owned()))
    }

    fn decision_args(collaboration_id: &str, workspace: &Path, scope: &str) -> Value {
        json!({
            "project_scope":"hcodex", "topic_title":"Cutover",
            "argon_architecture_intent":"Argon architectural intent approved.",
            "sonnet_craft_or_review_summary":"Sonnet surgical craft summary.",
            "codex_audit_or_synthesis_summary":"Codex audit verified CAS fix.",
            "final_resolution":"Adopt the CAS-guarded implementation.",
            "verification_evidence":"verified", "collaboration_id":collaboration_id,
            "workspace_dir":workspace, "expected_scope_digest":scope
        })
    }
}
