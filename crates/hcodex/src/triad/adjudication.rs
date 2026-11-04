use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::types::{Adjudication, RoleSnapshot, RoleState, RoleSummary, Verdict};

pub(crate) const REQUIRED_ROLES: [&str; 3] = ["argon", "sonnet", "codex"];

pub(crate) fn inspect_role(role: &str, response: Option<Value>) -> RoleSnapshot {
    let status = response.as_ref().and_then(response_status);
    let ok = response.as_ref().and_then(response_ok);
    let state = map_state(status.as_deref(), ok);
    let artifact = response.as_ref().and_then(extract_artifact);
    let narrative = artifact.as_ref().is_some_and(has_narrative);
    let verdict = artifact
        .as_ref()
        .and_then(extract_verdict)
        .filter(|_| narrative);
    let digest = artifact
        .as_ref()
        .and_then(|value| canonical_digest(value).ok());
    RoleSnapshot {
        role: role.to_owned(),
        state,
        verdict,
        response_present: response.is_some(),
        response_status: status,
        response_ok: ok,
        model_id: response.as_ref().and_then(model_id),
        job_id: response.as_ref().and_then(job_id),
        handoff_ticket_id: response
            .as_ref()
            .and_then(|value| value.get("handoff_ticket_id"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        review_artifact_present: artifact.is_some(),
        review_artifact_has_narrative: narrative,
        review_artifact_digest: digest,
        artifact,
        response,
    }
}

fn response_status(response: &Value) -> Option<String> {
    response
        .get("status")
        .and_then(Value::as_str)
        .or_else(|| {
            response
                .pointer("/result/receipt/status")
                .and_then(Value::as_str)
        })
        .map(str::to_ascii_lowercase)
}

fn response_ok(response: &Value) -> Option<bool> {
    response.get("ok").and_then(Value::as_bool).or_else(|| {
        response_status(response).map(|status| matches!(status.as_str(), "succeeded" | "completed"))
    })
}

fn map_state(status: Option<&str>, ok: Option<bool>) -> RoleState {
    match status.unwrap_or_default() {
        "timed_out_unknown" | "outcome_unknown" | "unknown" => RoleState::TimedOutUnknown,
        "queued" | "pending" => RoleState::Queued,
        "running" | "accepted" | "in_flight" => RoleState::Running,
        "succeeded" | "completed" if ok == Some(true) => RoleState::Succeeded,
        "" if ok == Some(true) => RoleState::Succeeded,
        "" if ok.is_none() => RoleState::NotStarted,
        _ => RoleState::Failed,
    }
}

fn extract_artifact(response: &Value) -> Option<Value> {
    let candidates = [
        response.get("structured_output"),
        response.get("data"),
        response.pointer("/result/structured_output"),
        response.pointer("/result/data"),
        response.pointer("/result/result/structured_output"),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|value| value.as_object().is_some_and(|object| !object.is_empty()))
        .cloned()
}

fn has_narrative(artifact: &Value) -> bool {
    ["summary", "executive_critique"].into_iter().any(|key| {
        artifact
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|text| !text.trim().is_empty())
    })
}

fn extract_verdict(artifact: &Value) -> Option<Verdict> {
    match artifact
        .get("verdict")
        .and_then(Value::as_str)?
        .trim()
        .to_ascii_uppercase()
        .as_str()
    {
        "PASS" => Some(Verdict::Pass),
        "WARN" => Some(Verdict::Warn),
        "BLOCK" => Some(Verdict::Block),
        _ => None,
    }
}

fn model_id(response: &Value) -> Option<String> {
    response
        .pointer("/result/receipt/actual_model")
        .or_else(|| response.get("model_id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn job_id(response: &Value) -> Option<String> {
    response
        .get("job_id")
        .or_else(|| response.get("request_id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

pub(crate) fn adjudicate(roles: &BTreeMap<String, RoleSnapshot>) -> Adjudication {
    let required_roles = REQUIRED_ROLES
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let succeeded_roles = REQUIRED_ROLES
        .iter()
        .filter(|role| {
            roles
                .get(**role)
                .is_some_and(|snapshot| snapshot.state == RoleState::Succeeded)
        })
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let incomplete_roles = REQUIRED_ROLES
        .iter()
        .filter(|role| !succeeded_roles.iter().any(|value| value == **role))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let collaboration_status = if succeeded_roles.len() == REQUIRED_ROLES.len() {
        "complete"
    } else if !succeeded_roles.is_empty() {
        "partial"
    } else if REQUIRED_ROLES.iter().any(|role| {
        roles.get(*role).is_some_and(|snapshot| {
            matches!(snapshot.state, RoleState::Queued | RoleState::Running)
        })
    }) {
        "pending"
    } else if REQUIRED_ROLES.iter().any(|role| {
        roles
            .get(*role)
            .is_some_and(|snapshot| snapshot.state == RoleState::TimedOutUnknown)
    }) {
        "timed_out_unknown"
    } else {
        "failed"
    }
    .to_owned();
    let missing_artifact_roles = missing(roles, |snapshot| snapshot.review_artifact_present);
    let missing_verdict_roles = missing(roles, |snapshot| snapshot.verdict.is_some());
    let missing_review_roles = missing(roles, |snapshot| {
        snapshot.state == RoleState::Succeeded
            && snapshot.review_artifact_present
            && snapshot.review_artifact_has_narrative
            && snapshot.verdict.is_some()
    });
    let verdicts = REQUIRED_ROLES
        .iter()
        .filter_map(|role| roles.get(*role).and_then(|snapshot| snapshot.verdict))
        .collect::<BTreeSet<_>>();
    let conflict_detected = verdicts.len() > 1;
    let (consensus_status, consensus_reached) =
        if collaboration_status != "complete" || !missing_review_roles.is_empty() {
            ("incomplete_no_consensus", false)
        } else if conflict_detected {
            ("conflict_requires_adjudication", false)
        } else if verdicts.contains(&Verdict::Block) {
            ("blocked_by_red_team", false)
        } else {
            ("consensus_verified", true)
        };
    let review_approved = consensus_reached
        && REQUIRED_ROLES.iter().all(|role| {
            roles.get(*role).and_then(|snapshot| snapshot.verdict) == Some(Verdict::Pass)
        });
    let role_states = REQUIRED_ROLES
        .iter()
        .map(|role| {
            let snapshot = roles.get(*role);
            (
                (*role).to_owned(),
                RoleSummary {
                    state: snapshot.map_or(RoleState::NotStarted, |value| value.state),
                    verdict: snapshot.and_then(|value| value.verdict),
                    has_artifact: snapshot.is_some_and(|value| value.review_artifact_present),
                    has_valid_review: !missing_review_roles
                        .iter()
                        .any(|missing_role| missing_role == *role),
                },
            )
        })
        .collect();
    let conflict_basis = conflict_detected.then(|| {
        REQUIRED_ROLES
            .iter()
            .filter_map(|role| {
                roles
                    .get(*role)
                    .and_then(|snapshot| snapshot.verdict)
                    .map(|verdict| format!("{role}={}", verdict_label(verdict)))
            })
            .collect::<Vec<_>>()
            .join(", ")
    });
    Adjudication {
        collaboration_status,
        consensus_status: consensus_status.to_owned(),
        consensus_reached,
        required_roles,
        succeeded_roles,
        incomplete_roles,
        role_states,
        missing_artifact_roles,
        missing_verdict_roles,
        missing_review_roles,
        conflict_detected,
        conflict_basis,
        decision_authority: "gemini-4-argon".to_owned(),
        review_approved,
    }
}

fn verdict_label(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Pass => "PASS",
        Verdict::Warn => "WARN",
        Verdict::Block => "BLOCK",
    }
}

fn missing<F>(roles: &BTreeMap<String, RoleSnapshot>, predicate: F) -> Vec<String>
where
    F: Fn(&RoleSnapshot) -> bool,
{
    REQUIRED_ROLES
        .iter()
        .filter(|role| !roles.get(**role).is_some_and(&predicate))
        .map(ToString::to_string)
        .collect()
}

pub(crate) fn canonical_digest(value: &Value) -> crate::Result<String> {
    let encoded = serde_json::to_vec(&canonicalize(value))?;
    let mut hasher = Sha256::new();
    hasher.update(encoded);
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

pub(crate) fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), canonicalize(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(canonicalize).collect()),
        _ => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn successful(role: &str, verdict: &str) -> RoleSnapshot {
        inspect_role(
            role,
            Some(
                json!({"ok":true,"status":"succeeded","job_id":format!("cdx-{role}"),"result":{"structured_output":{"verdict":verdict,"summary":"reviewed"},"receipt":{"actual_model":role}}}),
            ),
        )
    }

    #[test]
    fn incomplete_states_never_become_consensus() {
        for state in [
            RoleState::NotStarted,
            RoleState::Queued,
            RoleState::Running,
            RoleState::Failed,
            RoleState::TimedOutUnknown,
        ] {
            let mut roles = BTreeMap::from([
                ("sonnet".into(), successful("sonnet", "PASS")),
                ("codex".into(), successful("codex", "PASS")),
            ]);
            let mut argon = successful("argon", "PASS");
            argon.state = state;
            roles.insert("argon".into(), argon);
            let result = adjudicate(&roles);
            assert!(!result.consensus_reached);
            assert_eq!(result.consensus_status, "incomplete_no_consensus");
        }
    }

    #[test]
    fn distinguishes_block_conflict_and_approval() {
        let roles = |argon, sonnet, codex| {
            BTreeMap::from([
                ("argon".into(), successful("argon", argon)),
                ("sonnet".into(), successful("sonnet", sonnet)),
                ("codex".into(), successful("codex", codex)),
            ])
        };
        assert_eq!(
            adjudicate(&roles("BLOCK", "BLOCK", "BLOCK")).consensus_status,
            "blocked_by_red_team"
        );
        let conflict = adjudicate(&roles("PASS", "WARN", "PASS"));
        assert_eq!(conflict.consensus_status, "conflict_requires_adjudication");
        assert!(
            conflict
                .conflict_basis
                .as_deref()
                .is_some_and(|basis| basis.contains("sonnet=WARN"))
        );
        assert!(adjudicate(&roles("PASS", "PASS", "PASS")).review_approved);
        let unanimous_warn = adjudicate(&roles("WARN", "WARN", "WARN"));
        assert!(unanimous_warn.consensus_reached);
        assert_eq!(unanimous_warn.consensus_status, "consensus_verified");
        assert!(!unanimous_warn.review_approved);
    }

    #[test]
    fn canonical_digest_ignores_object_order() -> crate::Result<()> {
        assert_eq!(
            canonical_digest(&json!({"b":2,"a":1}))?,
            canonical_digest(&json!({"a":1,"b":2}))?
        );
        Ok(())
    }

    #[test]
    fn gateway_result_requires_nested_artifact_and_narrative() {
        let valid = inspect_role(
            "codex",
            Some(json!({
                "ok":true,
                "job_id":"cdx-role",
                "result":{
                    "receipt":{"status":"succeeded","actual_model":"gpt-6-astra"},
                    "structured_output":{"verdict":"PASS","executive_critique":"audited"}
                }
            })),
        );
        assert_eq!(valid.state, RoleState::Succeeded);
        assert_eq!(valid.verdict, Some(Verdict::Pass));
        assert_eq!(valid.model_id.as_deref(), Some("gpt-6-astra"));

        let no_narrative = inspect_role(
            "codex",
            Some(json!({"ok":true,"status":"succeeded","structured_output":{"verdict":"PASS"}})),
        );
        assert_eq!(no_narrative.verdict, None);
        assert!(
            !adjudicate(&BTreeMap::from([("codex".to_owned(), no_narrative)])).consensus_reached
        );

        let invalid_status = inspect_role(
            "codex",
            Some(
                json!({"ok":true,"status":"mystery","structured_output":{"verdict":"PASS","summary":"text"}}),
            ),
        );
        assert_eq!(invalid_status.state, RoleState::Failed);
    }
}
