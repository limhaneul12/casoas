use std::path::Path;

use serde_json::{Value, json};

use crate::Result;
use crate::gateway::Gateway;
use crate::storage::{JobRecord, JobStatus};

pub async fn status(gateway: &Gateway, original: &Path, flow_id: &str) -> Result<Value> {
    validate_id(flow_id)?;
    let state = gateway.state_root.clone();
    let original = original.canonicalize()?;
    let id = flow_id.to_owned();
    tokio::task::spawn_blocking(move || {
        use crate::storage::diagnostics::{ReadObservation, read};
        match read(&state, &id) {
            ReadObservation::Observed(snapshot) => {
                let Some(job) = snapshot.job else {
                    return Ok(json!({"ok":false,"read_only":true,"flow_id":id,"error":"workflow_not_found"}));
                };
                if job.task_type != "workspace_flow" || Path::new(&job.workspace_dir) != original {
                    return Err("workflow does not belong to this original workspace".into());
                }
                let mut value = job.result.clone().unwrap_or_else(|| json!({
                    "schema_version":"workspace_flow/v1",
                    "flow_id":job.job_id,
                    "workflow_status":workflow_status(job.status),
                }));
                if value.get("selected_stages").is_none() {
                    value["selected_stages"]=job.metadata.get("selected_stages").and_then(|raw|serde_json::from_str::<Value>(raw).ok()).unwrap_or_else(||json!([]));
                }
                if value.get("stages").and_then(Value::as_array).is_none_or(|stages|stages.is_empty()) {
                    value["stages"] = json!(expected_stages(&state, &job));
                }
                value["expected_child_ids"]=json!(["research","planning","building"].into_iter().filter_map(|kind|job.metadata.get(&format!("expected_{kind}_job_id")).map(|id|(kind,id))).collect::<std::collections::BTreeMap<_,_>>());
                value["expected_plan_artifact_id"] = json!(job.metadata.get("expected_plan_artifact_id"));
                value["flow_id"] = json!(job.job_id);
                value["job_status"] = json!(job.status);
                value["workspace_dir"] = json!(job.workspace_dir);
                value["deduplicated_existing_request"] = json!(true);
                value["read_only"] = json!(true);
                value["diagnostic_fingerprint"] = json!(snapshot.fingerprint);
                Ok(value)
            }
            ReadObservation::Unavailable(issue) | ReadObservation::Unknown(issue) => Ok(json!({
                "ok":false,"read_only":true,"flow_id":id,"error":issue.code()
            })),
        }
    })
    .await?
}

pub async fn cancel(gateway: &Gateway, original: &Path, flow_id: &str) -> Result<bool> {
    validate_id(flow_id)?;
    let original = original.canonicalize()?;
    let id = flow_id.to_owned();
    let observed = observe(&gateway.state_root, &original, &id).await?;
    let Some(observed) = observed else {
        return Ok(false);
    };
    if observed.metadata.get("owner_pid") != Some(&std::process::id().to_string()) {
        return Ok(false);
    }
    if !matches!(
        observed.status,
        JobStatus::Running | JobStatus::TimedOutUnknown
    ) {
        return Ok(false);
    }
    // PID equality alone cannot prove ownership after PID reuse or a crash.
    // Only the current Gateway's live supervisor control grants cancellation.
    if !gateway.cancel_supervised(flow_id).await {
        return Ok(false);
    }
    gateway
        .database(move |store| {
            store.cancel_job(
                &id,
                "workflow cancellation requested by current owner".into(),
            )?;
            Ok(())
        })
        .await?;
    Ok(true)
}

async fn observe(state: &Path, original: &Path, flow_id: &str) -> Result<Option<JobRecord>> {
    let state = state.to_path_buf();
    let original = original.to_path_buf();
    let id = flow_id.to_owned();
    tokio::task::spawn_blocking(move || {
        use crate::storage::diagnostics::{ReadObservation, read};
        match read(&state, &id) {
            ReadObservation::Observed(snapshot) => match snapshot.job {
                Some(job)
                    if job.task_type == "workspace_flow"
                        && Path::new(&job.workspace_dir) == original =>
                {
                    Ok(Some(job))
                }
                Some(_) => Err("workflow does not belong to this original workspace".into()),
                None => Ok(None),
            },
            ReadObservation::Unavailable(_) => Ok(None),
            ReadObservation::Unknown(issue) => Err(format!(
                "workflow cancellation observation unavailable: {}",
                issue.code()
            )
            .into()),
        }
    })
    .await?
}

fn workflow_status(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Running => "running",
        JobStatus::Completed | JobStatus::Succeeded => "completed",
        JobStatus::TimedOutUnknown => "timed_out_unknown",
        JobStatus::Cancelled => "cancelled",
        JobStatus::RejectedPreDispatch | JobStatus::InputBudgetExceeded | JobStatus::Failed => {
            "failed"
        }
    }
}

fn expected_stages(state: &Path, parent: &JobRecord) -> Vec<Value> {
    use crate::storage::diagnostics::{ReadObservation, read};
    ["research", "planning", "building"].into_iter().filter_map(|stage|{
        let id=parent.metadata.get(&format!("expected_{stage}_job_id"))?;
        let observed=match read(state,id) {
            ReadObservation::Observed(snapshot)=>snapshot.job,
            ReadObservation::Unavailable(_)=>None,
            ReadObservation::Unknown(issue)=>return Some(json!({"stage":stage,"job_id":id,"status":"timed_out_unknown","result":{"error":issue.code()}})),
        };
        let Some(job)=observed else {
            return Some(json!({"stage":stage,"job_id":id,"status":"blocked","result":{"not_admitted":true,"detail":"No durable child admission was observed"}}));
        };
        let mut result=job.result.clone().unwrap_or(Value::Null);
        let mut status=workflow_status(job.status);
        if matches!(job.status,JobStatus::Completed|JobStatus::Succeeded) {
            if stage=="research" {
                status=match result["research_status"].as_str(){Some("complete")=>"completed",Some("incomplete_evidence")=>"incomplete_evidence",Some("partial")=>"timed_out_unknown",_=>"stopped"};
            } else if stage=="planning" {
                status="stopped";
                if let Some(plan_id)=parent.metadata.get("expected_plan_artifact_id")
                    && let ReadObservation::Observed(snapshot)=read(state,plan_id)
                    && let Some(plan_job)=snapshot.job
                    && matches!(plan_job.status,JobStatus::Completed|JobStatus::Succeeded)
                    && plan_job.task_type=="workspace_plan" && plan_job.workspace_dir==parent.workspace_dir
                    && let Ok(plan)=super::artifacts::plan_from_record(&plan_job,Path::new(&parent.workspace_dir))
                    && let Some(value)=plan_job.result
                {
                    status=if plan.mechanically_ready(){"build_ready"}else{"completed"};
                    result=value;
                }
            } else if stage=="building" {
                status=if result["ok"]==true {"settled"}else{"stopped"};
            }
        }
        Some(json!({"stage":stage,"job_id":id,"job_status":job.status,"status":status,"result":result}))
    }).collect()
}

fn validate_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 256
        || value.chars().any(char::is_control)
        || value.split_whitespace().count() != 1
    {
        return Err("invalid workflow ID".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_status_is_read_only_and_does_not_initialize_state() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let state = temp.path().join("absent");
        let gateway = Gateway::new(state.clone());
        let value = status(&gateway, temp.path(), "flow-missing").await?;
        assert_eq!(value["read_only"], true);
        assert_eq!(value["error"], "missing_database");
        assert!(!state.exists());
        assert!(!cancel(&gateway, temp.path(), "flow-missing").await?);
        assert!(!state.exists());
        Ok(())
    }
}

#[cfg(test)]
mod restoration_tests {
    use super::*;
    use crate::storage::NewJob;
    use std::collections::BTreeMap;
    #[tokio::test]
    async fn unfinished_status_preserves_expected_ids_without_reconciling_or_cancelling_nonowned_work()
    -> Result<()> {
        for unknown in [false, true] {
            let temp = tempfile::tempdir()?;
            let original = temp.path().join("repo");
            std::fs::create_dir(&original)?;
            let gateway = Gateway::new(temp.path().join("state"));
            let source = original.clone();
            gateway
                .database(move |store| {
                    store.create_job(NewJob {
                        task_type: "workspace_flow".into(),
                        workspace_dir: source,
                        model: None,
                        metadata: BTreeMap::from([
                            (
                                "selected_stages".into(),
                                "[\"planning\",\"building\"]".into(),
                            ),
                            ("expected_planning_job_id".into(), "wsp-original".into()),
                            ("expected_building_job_id".into(), "wsb-original".into()),
                            ("expected_plan_artifact_id".into(), "wpa-original".into()),
                        ]),
                        job_id: Some("flow-original".into()),
                        idempotency_key: None,
                        provider: None,
                        engine: None,
                        created_at: None,
                    })?;
                    if unknown {
                        store.mark_timed_out_unknown(
                            "flow-original",
                            "lost owner".into(),
                            None,
                            None,
                        )?;
                    }
                    Ok(())
                })
                .await?;
            let before = gateway
                .database(|store| Ok(serde_json::to_value(store.get_job("flow-original")?)?))
                .await?;
            let value = status(&gateway, &original, "flow-original").await?;
            assert_eq!(
                value["workflow_status"],
                if unknown {
                    "timed_out_unknown"
                } else {
                    "running"
                }
            );
            assert_eq!(value["selected_stages"], json!(["planning", "building"]));
            assert_eq!(value["expected_child_ids"]["planning"], "wsp-original");
            assert_eq!(value["stages"][1]["job_id"], "wsb-original");
            assert_eq!(value["stages"][1]["status"], "blocked");
            assert!(!cancel(&gateway, &original, "flow-original").await?);
            let after = gateway
                .database(|store| Ok(serde_json::to_value(store.get_job("flow-original")?)?))
                .await?;
            assert_eq!(before, after);
            assert!(!gateway.state_root.join("workspaces").exists());
            gateway.shutdown().await?;
        }
        Ok(())
    }
}
