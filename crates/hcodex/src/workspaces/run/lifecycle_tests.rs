use super::*;
use crate::storage::{JobStatus, NewJob};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[tokio::test]
async fn a_new_explicit_flow_id_admits_a_fresh_run_of_the_same_brief() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let original = temp.path().join("repo");
    std::fs::create_dir(&original)?;
    let gateway = Gateway::new(temp.path().join("state"));
    let mut first = WorkflowRequest::new("Same brief", original, vec![WorkspaceKind::Planning]);
    first.request_id = Some("flow-first".into());
    let mut second = first.clone();
    second.request_id = Some("flow-second".into());
    let identity = request_identity(&first)?;
    let one = register(&gateway, &first, "flow-first", &identity).await?;
    let two = register(
        &gateway,
        &second,
        "flow-second",
        &request_identity(&second)?,
    )
    .await?;
    assert!(one.should_dispatch && two.should_dispatch);
    assert_ne!(one.record.job_id, two.record.job_id);
    assert!(
        !register(&gateway, &first, "flow-first", &identity)
            .await?
            .should_dispatch
    );
    gateway.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn imported_running_unknown_cancelled_and_partial_research_never_dispatch_planning()
-> Result<()> {
    for (index, status) in [
        JobStatus::Running,
        JobStatus::TimedOutUnknown,
        JobStatus::Cancelled,
        JobStatus::Completed,
    ]
    .into_iter()
    .enumerate()
    {
        let temp = tempfile::tempdir()?;
        let original = temp.path().join("repo");
        std::fs::create_dir(&original)?;
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let gateway = Gateway::new(temp.path().join("state")).with_test_provider(move |_, _| {
            observed.fetch_add(1, Ordering::SeqCst);
            Err("planner must not be called".into())
        });
        let source = original.clone();
        let id = format!("research-unavailable-{index}");
        let owned_id = id.clone();
        gateway.database(move|store|{
            let new=NewJob{task_type:"research_campaign".into(),workspace_dir:source,model:None,metadata:BTreeMap::new(),job_id:Some(owned_id.clone()),idempotency_key:None,provider:Some("hcodex_research".into()),engine:Some("research".into()),created_at:None};
            store.create_job(new)?;
            let report=json!({"schema_version":"research_report/v1","mode":"research","request_id":owned_id,"research_status":"partial","sources":[],"stages":[]});
            match status {
                JobStatus::Running=>{},
                JobStatus::TimedOutUnknown=>{store.mark_timed_out_unknown(&owned_id,"unknown".into(),Some(report),None)?;},
                JobStatus::Cancelled=>{store.cancel_job_with_result(&owned_id,"cancelled".into(),Some(report),None)?;},
                JobStatus::Completed=>{store.complete_job(&owned_id,report,None,None)?;},
                _=>return Err("invalid negative fixture state".into()),
            }
            Ok(())
        }).await?;
        let mut request = WorkflowRequest::new(
            "Plan with explicit prior evidence",
            original,
            vec![WorkspaceKind::Planning],
        );
        request.request_id = Some(format!("flow-import-{index}"));
        request.research_id = Some(id);
        let result = run(&gateway, request, None).await?;
        assert_eq!(result["workflow_status"], "failed", "{result}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(!gateway.state_root.join("workspaces").exists());
        gateway.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn current_nested_stage_is_drained_on_parent_cancel_or_deadline() -> Result<()> {
    for timed_out in [false, true] {
        let temp = tempfile::tempdir()?;
        let gateway = Gateway::new(temp.path().join("state"));
        let done = Arc::new(AtomicBool::new(false));
        let drained = done.clone();
        let child_gateway = gateway.clone();
        let parent_gateway = gateway.clone();
        let (admitted, admission) = tokio::sync::oneshot::channel();
        let parent = gateway.supervise(
            "flow-owned".into(),
            Box::pin(async move {
                let stage = child_gateway.supervise(
                    "child-owned".into(),
                    Box::pin(async move {
                        let _sent = admitted.send(());
                        crate::supervisor::cancelled().await;
                        drained.store(true, Ordering::SeqCst);
                        Ok((
                            WorkspaceRunStatus::Cancelled,
                            json!({"owned_child_closed":true}),
                        ))
                    }),
                );
                let deadline = if timed_out {
                    Instant::now()
                } else {
                    Instant::now() + Duration::from_secs(5)
                };
                await_stage(
                    &parent_gateway,
                    "flow-owned",
                    "child-owned",
                    deadline,
                    stage,
                )
                .await
            }),
        );
        let controller = async {
            admission.await?;
            if !timed_out {
                assert!(gateway.cancel_supervised("flow-owned").await);
            }
            Ok::<_, crate::Error>(())
        };
        let (result, controller) = tokio::join!(parent, controller);
        controller?;
        let (status, observation) = result?;
        assert_eq!(
            status,
            if timed_out {
                WorkspaceRunStatus::TimedOutUnknown
            } else {
                WorkspaceRunStatus::Cancelled
            }
        );
        assert!(
            observation["owned_child_closed"] == true
                || observation["observed_result"]["owned_child_closed"] == true,
            "{observation}"
        );
        assert!(done.load(Ordering::SeqCst));
        assert!(!can_continue(status));
        gateway.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn state_root_inside_original_is_rejected_before_store_or_stage_creation() -> Result<()> {
    let original = tempfile::tempdir()?;
    let state = original.path().join("must-stay-absent");
    let gateway = Gateway::new(state.clone());
    let mut request = WorkflowRequest::new(
        "build an explicit plan",
        original.path().into(),
        vec![WorkspaceKind::Building],
    );
    request.plan_id = Some("wpa-missing".into());
    assert!(run(&gateway, request, None).await.is_err());
    assert!(!state.exists());
    Ok(())
}
