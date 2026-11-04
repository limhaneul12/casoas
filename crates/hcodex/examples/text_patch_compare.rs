//! Replay frozen model outputs without changing the source under comparison.
use hcodex::dag::{PatchArtifact, TextPatch, TextPatchHunk};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Sample {
    task_id: String,
    mode: Mode,
    job_id: String,
    requested_model: String,
    actual_model: Option<String>,
    model_mismatch: bool,
    succeeded: bool,
    duration_ms: u64,
    input_bytes: Option<u64>,
    tokens_in: Option<u64>,
    tokens_out: Option<u64>,
    payload: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Full,
    Hunks,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Hunks => "hunks",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    model: String,
    source_sha256: String,
    source_bytes: usize,
    tasks: Vec<Task>,
    metrics: Vec<String>,
    limitations: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Task {
    id: String,
    instruction: String,
    changes: Vec<[String; 2]>,
    order: Vec<Mode>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HunkResponse {
    hunks: Vec<ProposedHunk>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposedHunk {
    start_line: usize,
    end_line: usize,
    replacement: String,
}

fn hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn compile(source: &str, mode: Mode, payload: &str) -> hcodex::Result<Vec<u8>> {
    match mode {
        Mode::Full => {
            let artifact: PatchArtifact = serde_json::from_str(payload)?;
            if artifact.edits.len() != 1 {
                return Err("expected exactly one full-file edit".into());
            }
            let edit = &artifact.edits[0];
            if edit.path != std::path::Path::new("source.rs")
                || edit.preimage_sha256 != hash(source.as_bytes())
            {
                return Err("full-file identity mismatch".into());
            }
            Ok(edit.new_content.as_bytes().to_vec())
        }
        Mode::Hunks => {
            let response: HunkResponse = serde_json::from_str(payload)?;
            // This frozen experiment uses an LF-only source. Hashes are supplied
            // by the host from that source, never calculated by the model.
            if source.contains('\r') {
                return Err("comparison fixture must be LF-only".into());
            }
            let lines: Vec<&str> = source.split_inclusive('\n').collect();
            let mut hunks = Vec::new();
            for proposed in response.hunks {
                let start = proposed.start_line.checked_sub(1).ok_or("line zero")?;
                let selected = lines
                    .get(start..proposed.end_line)
                    .filter(|slice| !slice.is_empty())
                    .ok_or("invalid original line range")?;
                hunks.push(TextPatchHunk {
                    start_line: proposed.start_line,
                    end_line: proposed.end_line,
                    range_sha256: hash(selected.concat().as_bytes()),
                    replacement: proposed.replacement,
                });
            }
            Ok(TextPatch {
                preimage_sha256: hash(source.as_bytes()),
                hunks,
            }
            .compile(source.as_bytes())?
            .content)
        }
    }
}

fn verify_record(
    root: &Path,
    sample: &Sample,
    source: &str,
    task: &Task,
) -> hcodex::Result<String> {
    let base = format!("{}-{}", sample.task_id, sample.mode.name());
    let bytes = fs::read(root.join(format!("{base}-receipt.json")))?;
    let raw: Value = serde_json::from_slice(&bytes)?;
    let job = &raw["job"];
    let receipt = &job["receipt"];
    let result = &job["result"];
    let observed: Sample = serde_json::from_value(json!({
        "task_id": sample.task_id, "mode": sample.mode,
        "job_id": job["job_id"], "requested_model":receipt["requested_model"],
        "actual_model":receipt["actual_model"], "model_mismatch":receipt["model_mismatch"],
        "succeeded":receipt["status"] == "succeeded", "duration_ms":receipt["duration_ms"],
        "input_bytes":result["budget"]["actual_input_bytes"],
        "tokens_in":receipt["tokens_in"], "tokens_out":receipt["tokens_out"],
        "payload":result["structured_output"]["crafted_code"], "error":job["error"],
    }))?;
    if observed != *sample {
        return Err("sample projection differs from original bridge receipt".into());
    }
    let request_bytes = fs::read(root.join(format!("{base}-request.json")))?;
    let request: Value = serde_json::from_slice(&request_bytes)?;
    let context = format!(
        "Frozen source.rs, exact whole-file preimage {}. Line numbers in the listing are NOT source bytes. Source has a final LF.\n{}",
        hash(source.as_bytes()),
        source
            .lines()
            .enumerate()
            .map(|(i, line)| format!("{}\t{line}", i + 1))
            .collect::<Vec<_>>()
            .join("\n")
    );
    // The bridge hashes target context followed by task_spec (builder::inline_context).
    let task_spec = request["task_spec"].as_str().ok_or("missing task spec")?;
    let inline = format!("{context}\n{task_spec}");
    if request["target_code_or_context"].as_str() != Some(&context)
        || result["evidence_scope"]["inline_context_digest"] != hash(inline.as_bytes())
        || request["claude_model"].as_str() != Some(&sample.requested_model)
        || !request["task_spec"]
            .as_str()
            .is_some_and(|spec| spec.contains(&task.instruction))
    {
        return Err(
            "request source/task/model differs from frozen plan or bridge context digest".into(),
        );
    }
    Ok(hash(&bytes))
}

fn main() -> hcodex::Result<()> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("comparison directory required")?,
    );
    let source = fs::read_to_string(root.join("source.txt"))?;
    let plan_bytes = fs::read(root.join("plan.json"))?;
    let plan: Plan = serde_json::from_slice(&plan_bytes)?;
    if plan.source_sha256 != hash(source.as_bytes()).trim_start_matches("sha256:")
        || plan.source_bytes != source.len()
        || plan.tasks.len() != 3
    {
        return Err("source differs from frozen plan".into());
    }
    let samples: Vec<Sample> = serde_json::from_slice(&fs::read(root.join("samples.json"))?)?;
    let planned: Vec<_> = plan
        .tasks
        .iter()
        .flat_map(|task| task.order.iter().map(|mode| (task.id.clone(), *mode)))
        .collect();
    let observed: Vec<_> = samples
        .iter()
        .map(|sample| (sample.task_id.clone(), sample.mode))
        .collect();
    if planned.len() != 6
        || planned.iter().collect::<BTreeSet<_>>().len() != 6
        || observed != planned
    {
        return Err(
            "samples must cover exactly six planned unique cells in predeclared order".into(),
        );
    }
    let mut rows = Vec::new();
    for sample in samples {
        if !["one_line", "disjoint", "block"].contains(&sample.task_id.as_str()) {
            return Err("unexpected comparison task".into());
        }
        if sample.requested_model != plan.model {
            return Err("sample model differs from frozen plan".into());
        }
        let task = plan
            .tasks
            .iter()
            .find(|task| task.id == sample.task_id)
            .ok_or("task absent from plan")?;
        let expected = fs::read(root.join(format!("{}-expected.txt", sample.task_id)))?;
        let mut reconstructed = source.clone();
        for [old, new] in &task.changes {
            if reconstructed.matches(old).count() != 1 {
                return Err("nonunique expected edit".into());
            }
            reconstructed = reconstructed.replacen(old, new, 1);
        }
        if expected != reconstructed.as_bytes() {
            return Err("expected bytes differ from planned changes".into());
        }
        let receipt_digest = verify_record(&root, &sample, &source, task)?;
        let result = match sample.payload.as_deref() {
            Some(payload)
                if !sample.model_mismatch
                    && sample.actual_model.as_deref() == Some(&sample.requested_model) =>
            {
                compile(&source, sample.mode, payload)
            }
            _ => Err("provider did not confirm requested-model success".into()),
        };
        let compiled = result.is_ok();
        let exact = result.as_ref().is_ok_and(|bytes| *bytes == expected);
        rows.push(json!({
            "task": sample.task_id,
            "mode": match sample.mode { Mode::Full => "full", Mode::Hunks => "hunks" },
            "job_id": sample.job_id,
            "receipt_sha256": receipt_digest,
            "expected_sha256": hash(&expected),
            "actual_model": sample.actual_model,
            "bridge_accepted": sample.succeeded,
            "compiled": compiled,
            "exact": exact,
            "accepted_exact": sample.succeeded && exact,
            "payload_bytes": sample.payload.as_ref().map(String::len),
            "duration_ms": sample.duration_ms,
            "input_bytes": sample.input_bytes,
            "tokens_in_reported": sample.tokens_in,
            "tokens_out_reported": sample.tokens_out,
            "rejection": result.err().map(|error| error.to_string()).or(sample.error),
        }));
    }
    let report = json!({"plan_sha256":hash(&plan_bytes),"source_sha256": hash(source.as_bytes()), "metrics":plan.metrics,"limitations":plan.limitations,"rows": rows});
    let rendered = serde_json::to_string_pretty(&report)?;
    fs::write(root.join("comparison.json"), &rendered)?;
    println!("{rendered}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_formats_produce_same_exact_bytes_and_bad_output_is_not_repaired() -> hcodex::Result<()>
    {
        let source = "a\nb\nc\n";
        let full = json!({"edits":[{"path":"source.rs","preimage_sha256":hash(source.as_bytes()),"new_content":"A\nb\nC\n"}]}).to_string();
        let hunks = r#"{"hunks":[{"start_line":1,"end_line":1,"replacement":"A\n"},{"start_line":3,"end_line":3,"replacement":"C\n"}]}"#;
        assert_eq!(compile(source, Mode::Full, &full)?, b"A\nb\nC\n");
        assert_eq!(compile(source, Mode::Hunks, hunks)?, b"A\nb\nC\n");
        assert!(
            compile(
                source,
                Mode::Hunks,
                &hunks.replace("\"end_line\":3", "\"end_line\":30")
            )
            .is_err()
        );
        assert!(compile(source, Mode::Full, &full.replace("source.rs", "other.rs")).is_err());
        assert!(
            compile(
                source,
                Mode::Full,
                &full.replace(&hash(source.as_bytes()), &hash(b"stale"))
            )
            .is_err()
        );
        Ok(())
    }
}
