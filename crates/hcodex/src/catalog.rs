//! Pinned MCP wire contracts captured from the previous registered server.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::LazyLock;

static TOOLS: LazyLock<Option<Vec<ToolDefinition>>> =
    LazyLock::new(|| serde_json::from_str(include_str!("../mcp-tools.json")).ok());

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
    #[serde(flatten)]
    pub metadata: Map<String, Value>,
}

pub fn tools() -> crate::Result<Vec<ToolDefinition>> {
    TOOLS
        .as_ref()
        .cloned()
        .ok_or_else(|| "invalid embedded mcp-tools.json".into())
}

/// Rejects unknown arguments and applies the previous server's defaults.
pub fn arguments(name: &str, value: Value) -> crate::Result<Value> {
    let tools = TOOLS.as_ref().ok_or("invalid embedded mcp-tools.json")?;
    let tool = tools
        .iter()
        .find(|tool| tool.name == name)
        .ok_or_else(|| format!("unknown tool: {name}"))?;
    let Value::Object(mut args) = value else {
        return Err("arguments must be an object".into());
    };
    let properties = tool.input_schema["properties"]
        .as_object()
        .ok_or("missing properties")?;
    for key in args.keys() {
        if !properties.contains_key(key) {
            return Err(format!("unknown argument: {key}").into());
        }
    }
    if let Some(required) = tool.input_schema["required"].as_array() {
        for key in required.iter().filter_map(Value::as_str) {
            if !args.contains_key(key) {
                return Err(format!("required argument: {key}").into());
            }
        }
    }
    for (key, schema) in properties {
        if let Some(value) = args.get(key) {
            validate(value, schema, &tool.input_schema, key, 0)?;
        } else if let Some(default) = schema.get("default") {
            args.insert(key.clone(), default.clone());
        }
    }
    Ok(Value::Object(args))
}

/// Validates the pinned wire/schema subset including references and nested closed objects.
pub fn validate_schema(value: &Value, schema: &Value) -> crate::Result<()> {
    validate(value, schema, schema, "$", 0)
}
fn validate(
    value: &Value,
    schema: &Value,
    root: &Value,
    path: &str,
    depth: usize,
) -> crate::Result<()> {
    if depth > 128 {
        return Err("schema nesting limit exceeded".into());
    }
    if schema == &Value::Bool(false) {
        return Err(format!("forbidden value: {path}").into());
    }
    if let Some(reference) = schema["$ref"].as_str() {
        let pointer = reference
            .strip_prefix('#')
            .ok_or("external schema references are forbidden")?;
        let resolved = root.pointer(pointer).ok_or("unresolved schema reference")?;
        return validate(value, resolved, root, path, depth + 1);
    }
    if let Some(choices) = schema["anyOf"].as_array()
        && !choices
            .iter()
            .any(|choice| validate(value, choice, root, path, depth + 1).is_ok())
    {
        return Err(format!("invalid schema choice: {path}").into());
    }
    let valid = match &schema["type"] {
        Value::Null => true,
        Value::String(kind) => matches_type_name(value, kind)?,
        Value::Array(kinds) => {
            let mut matched = false;
            for kind in kinds {
                let name = kind
                    .as_str()
                    .ok_or_else(|| format!("unsupported schema type in array at {path}"))?;
                if matches_type_name(value, name)? {
                    matched = true;
                    break;
                }
            }
            matched
        }
        other => return Err(format!("unsupported schema type: {other}").into()),
    };
    if !valid {
        return Err(format!("invalid value type: {path}").into());
    }
    if let Some(choices) = schema["enum"].as_array()
        && !choices.contains(value)
    {
        return Err(format!("invalid enum: {path}").into());
    }
    if let Some(constant) = schema.get("const")
        && constant != value
    {
        return Err(format!("constant mismatch: {path}").into());
    }
    if let Some(number) = value.as_f64()
        && (schema["minimum"].as_f64().is_some_and(|min| number < min)
            || schema["maximum"].as_f64().is_some_and(|max| number > max))
    {
        return Err(format!("number out of range: {path}").into());
    }
    if let Some(text) = value.as_str() {
        let len = u64::try_from(text.chars().count())?;
        if schema["minLength"].as_u64().is_some_and(|min| len < min)
            || schema["maxLength"].as_u64().is_some_and(|max| len > max)
        {
            return Err(format!("string length out of range: {path}").into());
        }
    }
    if let Some(items) = value.as_array() {
        let len = u64::try_from(items.len())?;
        if schema["minItems"].as_u64().is_some_and(|min| len < min)
            || schema["maxItems"].as_u64().is_some_and(|max| len > max)
        {
            return Err(format!("array length out of range: {path}").into());
        }
        for (index, item) in items.iter().enumerate() {
            validate(
                item,
                &schema["items"],
                root,
                &format!("{path}/{index}"),
                depth + 1,
            )?;
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = schema["required"].as_array() {
            for key in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(key) {
                    return Err(format!("required field: {path}/{key}").into());
                }
            }
        }
        for (key, item) in object {
            if let Some(property) = schema.get("properties").and_then(|props| props.get(key)) {
                validate(item, property, root, &format!("{path}/{key}"), depth + 1)?;
            } else if let Some(additional) = schema.get("additionalProperties") {
                validate(item, additional, root, &format!("{path}/{key}"), depth + 1)?;
            }
        }
    }
    Ok(())
}

fn matches_type_name(value: &Value, kind: &str) -> crate::Result<bool> {
    Ok(match kind {
        "null" => value.is_null(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        other => return Err(format!("unsupported schema type: {other}").into()),
    })
}

#[cfg(test)]
mod tests;
