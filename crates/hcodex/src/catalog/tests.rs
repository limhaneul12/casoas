use super::*;
use serde_json::json;

#[test]
fn referenced_nested_objects_are_closed_and_bounded() -> crate::Result<()> {
    let schema = json!({"type":"object","required":["node"],"additionalProperties":false,"properties":{"node":{"$ref":"#/$defs/Node"}},"$defs":{"Node":{"type":"object","required":["count"],"additionalProperties":false,"properties":{"count":{"type":"integer","minimum":1,"maximum":8}}}}});
    validate_schema(&json!({"node":{"count":4}}), &schema)?;
    for invalid in [
        json!({"node":{"count":9}}),
        json!({"node":{"count":1,"extra":true}}),
        json!({"node":{}}),
    ] {
        assert!(validate_schema(&invalid, &schema).is_err());
    }
    Ok(())
}

#[test]
fn all_tools_and_closed_input_contract() -> crate::Result<()> {
    assert_eq!(tools()?.len(), 16);
    assert!(arguments("codex_job_status", json!({"surprise":true})).is_err());
    assert!(arguments("codex_adversarial_audit", json!({})).is_err());
    assert!(arguments("codex_job_list", json!({"limit":"bad"})).is_err());
    assert_eq!(arguments("codex_job_list", json!({}))?["limit"], 15);
    Ok(())
}
