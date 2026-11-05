//! One research integrity validator for runtime and the deterministic harness.
//! Quote fidelity is not semantic entailment or factual truth.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Component;

use chrono::DateTime;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::types::*;
use crate::Result;

pub const SEMANTIC_VERIFICATION: &str = "NOT_MECHANICALLY_PROVEN";
pub const HARNESS_TEXT: &str = include_str!("../../assets/research-rules.md");
pub const MAX_ARTIFACT_BYTES: usize = 262_144;

pub fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}
pub fn harness_sha256() -> String {
    sha256(HARNESS_TEXT.as_bytes())
}

pub fn validate_request(request: &ResearchRequest) -> Result<()> {
    bounded_text(&request.question, 16_384, "research question")?;
    bounded_text(&request.output_language, 64, "output language")?;
    if let Some(as_of) = &request.as_of {
        chrono::NaiveDate::parse_from_str(as_of, "%Y-%m-%d")
            .map_err(|_| "as_of must be an ISO calendar date")?;
    }
    for id in [&request.request_id, &request.idempotency_key]
        .into_iter()
        .flatten()
    {
        identifier(id, 256)?;
    }
    let b = &request.budget;
    if !(1..=3).contains(&b.max_parallel)
        || !(1..=24).contains(&b.max_sources)
        || !(1..=1_048_576).contains(&b.max_source_bytes)
        || !(1..=4_194_304).contains(&b.max_total_source_bytes)
        || !(1..=16_384).contains(&b.max_excerpt_bytes)
        || !(1..=131_072).contains(&b.max_total_excerpt_bytes)
        || !(1..=64).contains(&b.max_claims)
        || b.timeout_seconds > 3_600
        || b.max_source_bytes > b.max_total_source_bytes
        || b.max_excerpt_bytes > b.max_total_excerpt_bytes
        || b.max_excerpt_bytes > b.max_source_bytes
    {
        return Err("research budget exceeds its finite contract".into());
    }
    if !(1..=3).contains(&request.investigators.len()) || request.sources.len() > b.max_sources {
        return Err("research investigator/source count exceeds budget".into());
    }
    let mut lanes = BTreeSet::new();
    for lane in &request.investigators {
        identifier(&lane.lane_id, 64)?;
        if !lanes.insert(&lane.lane_id) {
            return Err("duplicate research lane ID".into());
        }
        bounded_text(&lane.focus, 4_096, "research focus")?;
        validate_model(&lane.model, &lane.effort)?;
    }
    validate_model(&request.challenger.model, &request.challenger.effort)?;
    validate_model(&request.synthesizer.model, &request.synthesizer.effort)?;
    if let Some(DiscoveryConfig::Web {
        model,
        effort,
        max_search_events,
        max_candidates,
        min_accepted_sources,
        timeout_seconds,
    }) = request.active_discovery()
    {
        validate_model(&model, &effort)?;
        if super::execution_engine(&model) != "cli"
            || !(1..=MAX_SEARCH_EVENTS).contains(&max_search_events)
            || !(1..=MAX_DISCOVERY_CANDIDATES).contains(&max_candidates)
            || !(1..=max_candidates).contains(&min_accepted_sources)
            || min_accepted_sources > b.max_sources.saturating_sub(request.sources.len())
            || timeout_seconds > 120
        {
            return Err(
                "research discovery exceeds its native CLI or finite budget contract".into(),
            );
        }
    }
    let mut sources = BTreeSet::new();
    for source in &request.sources {
        let locator = normalized_locator(source)?;
        if !sources.insert(locator) {
            return Err("duplicate research source locator".into());
        }
    }
    Ok(())
}

fn validate_model(model: &str, effort: &str) -> Result<()> {
    bounded_text(model, 128, "research model")?;
    if !matches!(effort, "low" | "medium" | "high" | "xhigh" | "max") {
        return Err("invalid research model effort".into());
    }
    Ok(())
}

fn normalized_locator(source: &SourceSpec) -> Result<String> {
    match source {
        SourceSpec::Repository { path } | SourceSpec::TechnicalDocument { path } => {
            if path.as_os_str().is_empty()
                || path.is_absolute()
                || path
                    .components()
                    .any(|part| !matches!(part, Component::Normal(_)))
            {
                return Err("research local source must be a confined relative path".into());
            }
            if path.components().any(|part| {
                let name = part.as_os_str().to_string_lossy().to_ascii_lowercase();
                matches!(
                    name.as_str(),
                    ".git"
                        | ".hcodex"
                        | ".codex"
                        | ".omx"
                        | ".codegraph"
                        | ".ssh"
                        | ".aws"
                        | ".azure"
                        | ".gemini"
                        | ".envrc"
                        | ".netrc"
                        | ".npmrc"
                        | ".pypirc"
                        | "auth.json"
                        | "credentials"
                        | "credentials.json"
                        | "credentials.toml"
                        | "secrets.toml"
                        | "id_rsa"
                        | "id_ed25519"
                ) || name == ".env"
                    || name.starts_with(".env.")
                    || name.ends_with(".pem")
                    || name.ends_with(".key")
            }) {
                return Err("research input cannot select private state or credentials".into());
            }
            let text = path.to_str().ok_or("research source path must be UTF-8")?;
            bounded_text(text, 2_048, "research source path")?;
            Ok(text.to_owned())
        }
        SourceSpec::Web { url } => normalized_web_url(url),
    }
}

pub(crate) fn normalized_web_url(value: &str) -> Result<String> {
    bounded_text(value, 2_048, "research URL")?;
    let mut url = url::Url::parse(value)?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url
            .query_pairs()
            .any(|(key, _)| super::sources::sensitive_query(&key))
    {
        return Err("research web source requires credential-free HTTPS".into());
    }
    url.set_fragment(None);
    Ok(url.to_string())
}

pub fn validate_sources(request: &ResearchRequest, bundle: &SourceBundle) -> Result<()> {
    if bundle.sources.len() + bundle.failures.len() != request.sources.len()
        || bundle.sources.len() > request.budget.max_sources
    {
        return Err("source receipts do not account for the declared acquisition scope".into());
    }
    let mut ids = BTreeSet::new();
    let mut excerpts = BTreeSet::new();
    let mut locators = BTreeSet::new();
    let mut total_body = 0_u64;
    let mut total_excerpt = 0_usize;
    for source in &bundle.sources {
        identifier(&source.source_id, 64)?;
        identifier(&source.excerpt.excerpt_id, 96)?;
        if !ids.insert(&source.source_id)
            || !excerpts.insert(&source.excerpt.excerpt_id)
            || !locators.insert(&source.locator)
        {
            return Err("duplicate source, excerpt or captured locator".into());
        }
        if !declared_source(request, source)? {
            return Err("captured source is outside declared scope".into());
        }
        DateTime::parse_from_rfc3339(&source.observed_at)
            .map_err(|_| "invalid source observation timestamp")?;
        digest_shape(&source.body_sha256)?;
        let excerpt = &source.excerpt;
        bounded_text(
            &excerpt.text,
            request.budget.max_excerpt_bytes,
            "source excerpt",
        )?;
        if excerpt.source_id != source.source_id
            || excerpt.sha256 != sha256(excerpt.text.as_bytes())
            || excerpt.start_byte > excerpt.end_byte
            || excerpt.end_byte > source.body_bytes
            || excerpt.end_byte - excerpt.start_byte != excerpt.text.len() as u64
            || excerpt.start_line == 0
            || excerpt.end_line < excerpt.start_line
            || excerpt.end_line - excerpt.start_line != excerpt.text.lines().count().max(1) - 1
        {
            return Err("source excerpt digest, ownership or range mismatch".into());
        }
        if excerpt.start_byte == 0
            && excerpt.end_byte == source.body_bytes
            && source.body_sha256 != excerpt.sha256
        {
            return Err("complete source excerpt differs from observed body digest".into());
        }
        if source.body_bytes > request.budget.max_source_bytes as u64 {
            return Err("source body exceeds byte budget".into());
        }
        total_body = total_body
            .checked_add(source.body_bytes)
            .ok_or("source byte count overflow")?;
        total_excerpt = total_excerpt
            .checked_add(excerpt.text.len())
            .ok_or("excerpt byte count overflow")?;
    }
    if total_body > request.budget.max_total_source_bytes as u64
        || total_excerpt > request.budget.max_total_excerpt_bytes
    {
        return Err("aggregate source budget exceeded".into());
    }
    for failure in &bundle.failures {
        bounded_text(&failure.locator, 2_048, "failed source locator")?;
        bounded_text(&failure.error_code, 128, "source failure code")?;
        bounded_text(&failure.detail, 4_096, "source failure detail")?;
    }
    digest_shape(&bundle.snapshot_digest)?;
    if bundle.snapshot_digest != sha256(&serde_json::to_vec(&(&bundle.sources, &bundle.failures))?)
    {
        return Err("source snapshot digest differs from captured receipts".into());
    }
    Ok(())
}

fn declared_source(request: &ResearchRequest, captured: &SourceRecord) -> Result<bool> {
    for declared in &request.sources {
        if declared.kind() != captured.kind {
            continue;
        }
        match declared {
            SourceSpec::Repository { path } | SourceSpec::TechnicalDocument { path } => {
                if captured.locator == path.to_string_lossy()
                    || captured.locator == request.workspace_dir.join(path).to_string_lossy()
                {
                    return Ok(true);
                }
            }
            SourceSpec::Web { url } => {
                let expected = normalized_web_url(url)?;
                let origin = captured.redirect_chain.first().unwrap_or(&captured.locator);
                if normalized_web_url(origin)? == expected {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

pub fn validate_stage(
    phase: ResearchPhase,
    artifact: &StageArtifact,
    bundle: &SourceBundle,
    prior: &[ResearchClaim],
    budget: &ResearchBudget,
) -> Result<()> {
    if serde_json::to_vec(artifact)?.len() > MAX_ARTIFACT_BYTES {
        return Err("research stage artifact exceeds byte budget".into());
    }
    bounded_optional_text(&artifact.agent_reported_advisory, 4_096, "advisory summary")?;
    if artifact.claims.len() > budget.max_claims
        || prior.len() > budget.max_claims
        || artifact.challenges.len() > budget.max_claims
        || artifact.report_sections.len() > 32
        || artifact.unresolved.len() > 64
    {
        return Err("research artifact count exceeds budget".into());
    }
    for question in &artifact.unresolved {
        bounded_text(question, 2_048, "unresolved question")?;
    }
    let prior_map = claim_map(prior)?;
    match phase {
        ResearchPhase::Investigate => {
            if !artifact.challenges.is_empty()
                || !artifact.report_sections.is_empty()
                || (artifact.claims.is_empty() && artifact.unresolved.is_empty())
            {
                return Err("investigator must return claims or unresolved questions only".into());
            }
            let _claims = claim_map(&artifact.claims)?;
            for claim in &artifact.claims {
                validate_claim(claim, bundle)?;
            }
        }
        ResearchPhase::Challenge => {
            if !artifact.claims.is_empty() || !artifact.report_sections.is_empty() {
                return Err("challenger cannot create claims or report prose".into());
            }
            if artifact.challenges.is_empty()
                && artifact.unresolved.is_empty()
                && artifact.agent_reported_advisory.trim().is_empty()
            {
                return Err("challenger must return an observed review artifact".into());
            }
            let mut challenged = BTreeSet::new();
            for challenge in &artifact.challenges {
                let original = prior_map
                    .get(challenge.claim_id.as_str())
                    .ok_or("challenge references unknown claim")?;
                if !challenged.insert(&challenge.claim_id) {
                    return Err("duplicate claim challenge".into());
                }
                bounded_text(&challenge.reason, 4_096, "challenge reason")?;
                if rank(challenge.assessment) < rank(original.assessment) {
                    return Err("challenger cannot upgrade claim assessment".into());
                }
                validate_refs(&challenge.evidence_refs, bundle)?;
                if challenge.assessment == Assessment::Refuted
                    && !challenge
                        .evidence_refs
                        .iter()
                        .any(|r| r.stance == EvidenceStance::Counter)
                {
                    return Err("refutation requires captured counter-evidence".into());
                }
            }
            if challenged.len() != prior_map.len() {
                return Err("challenger must assess every prior claim exactly once".into());
            }
        }
        ResearchPhase::Synthesize => {
            if !artifact.claims.is_empty() || !artifact.challenges.is_empty() {
                return Err(
                    "synthesizer must reference existing claims, not generate facts".into(),
                );
            }
            let mut selected = BTreeSet::new();
            for section in &artifact.report_sections {
                bounded_text(&section.heading, 512, "report section heading")?;
                if section.claim_ids.is_empty() || section.claim_ids.len() > budget.max_claims {
                    return Err("invalid report section claim count".into());
                }
                for id in &section.claim_ids {
                    let claim = prior_map
                        .get(id.as_str())
                        .ok_or("report references unknown claim")?;
                    if !report_eligible(claim.assessment) {
                        return Err("unresolved claim cannot enter report body".into());
                    }
                    if !selected.insert(id) {
                        return Err("duplicate report claim reference".into());
                    }
                    validate_claim(claim, bundle)?;
                }
            }
            if artifact.report_sections.is_empty() && artifact.unresolved.is_empty() {
                return Err(
                    "synthesis requires report sections or explicit unresolved questions".into(),
                );
            }
            if prior.iter().any(|claim| {
                report_eligible(claim.assessment) && !selected.contains(&claim.claim_id)
            }) {
                return Err("synthesis omitted an eligible source-bound finding".into());
            }
        }
    }
    Ok(())
}

fn report_eligible(assessment: Assessment) -> bool {
    match assessment {
        Assessment::Supported | Assessment::Contested | Assessment::Refuted => true,
        Assessment::Unresolved => false,
    }
}

fn rank(value: Assessment) -> u8 {
    match value {
        Assessment::Supported => 0,
        Assessment::Contested => 1,
        Assessment::Unresolved => 2,
        Assessment::Refuted => 3,
    }
}

fn claim_map(claims: &[ResearchClaim]) -> Result<BTreeMap<&str, &ResearchClaim>> {
    let mut map = BTreeMap::new();
    for claim in claims {
        identifier(&claim.claim_id, 192)?;
        if map.insert(claim.claim_id.as_str(), claim).is_some() {
            return Err("duplicate research claim ID".into());
        }
    }
    Ok(map)
}

fn validate_claim(claim: &ResearchClaim, bundle: &SourceBundle) -> Result<()> {
    bounded_text(&claim.text, 8_192, "research claim")?;
    if matches!(
        claim.assessment,
        Assessment::Supported | Assessment::Contested
    ) && !claim
        .evidence_refs
        .iter()
        .any(|r| r.stance == EvidenceStance::Support)
    {
        return Err("supported or contested claim requires captured support".into());
    }
    if claim.assessment == Assessment::Refuted
        && !claim
            .evidence_refs
            .iter()
            .any(|r| r.stance == EvidenceStance::Counter)
    {
        return Err("refuted claim requires captured counter-evidence".into());
    }
    validate_refs(&claim.evidence_refs, bundle)
}

fn validate_refs(refs: &[EvidenceRef], bundle: &SourceBundle) -> Result<()> {
    if refs.len() > 24 {
        return Err("claim evidence reference budget exceeded".into());
    }
    let mut seen = BTreeSet::new();
    for evidence in refs {
        let source = bundle
            .sources
            .iter()
            .find(|s| s.source_id == evidence.source_id)
            .ok_or("claim references uncaptured source")?;
        let excerpt = &source.excerpt;
        if excerpt.excerpt_id != evidence.excerpt_id
            || excerpt.source_id != source.source_id
            || excerpt.sha256 != sha256(excerpt.text.as_bytes())
        {
            return Err("claim excerpt identity or digest mismatch".into());
        }
        bounded_text(&evidence.quote, 16_384, "evidence quote")?;
        let normalized = normalize_quote(&evidence.quote);
        if !normalize_quote(&excerpt.text).contains(&normalized) {
            return Err("claim quote is absent from captured excerpt".into());
        }
        if !seen.insert((
            &evidence.source_id,
            &evidence.excerpt_id,
            normalized,
            evidence.stance == EvidenceStance::Counter,
        )) {
            return Err("duplicate claim evidence reference".into());
        }
    }
    Ok(())
}

pub fn apply_challenges(claims: &mut [ResearchClaim], artifact: &StageArtifact) -> Result<()> {
    for challenge in &artifact.challenges {
        let claim = claims
            .iter_mut()
            .find(|c| c.claim_id == challenge.claim_id)
            .ok_or("challenge references unknown claim")?;
        if rank(challenge.assessment) < rank(claim.assessment) {
            return Err("challenger cannot upgrade claim assessment".into());
        }
        claim.assessment = challenge.assessment;
        for reference in &challenge.evidence_refs {
            if !claim.evidence_refs.contains(reference) {
                claim.evidence_refs.push(reference.clone());
            }
        }
    }
    Ok(())
}

pub fn render_report(
    claims: &[ResearchClaim],
    sections: &[ReportSection],
    unresolved: &[String],
    bundle: &SourceBundle,
) -> Result<String> {
    let map = claim_map(claims)?;
    let mut markdown = String::from(
        "# Research report\n\nCitation integrity checks captured excerpts; semantic truth is NOT_MECHANICALLY_PROVEN.\n",
    );
    let mut selected = BTreeSet::new();
    for section in sections {
        markdown.push_str(&format!("\n## {}\n", safe_text(&section.heading)));
        for id in &section.claim_ids {
            let claim = map
                .get(id.as_str())
                .ok_or("report references unknown claim")?;
            if !report_eligible(claim.assessment) {
                return Err("unresolved claim cannot enter report body".into());
            }
            selected.insert(id.as_str());
            render_claim(&mut markdown, claim, bundle)?;
        }
    }
    markdown.push_str("\n## Unresolved, refuted and omitted findings\n");
    for claim in claims
        .iter()
        .filter(|c| c.assessment == Assessment::Refuted || !selected.contains(c.claim_id.as_str()))
    {
        render_claim(&mut markdown, claim, bundle)?;
    }
    for item in unresolved {
        markdown.push_str(&format!("- {}\n", safe_text(item)));
    }
    Ok(markdown)
}

fn render_claim(markdown: &mut String, claim: &ResearchClaim, bundle: &SourceBundle) -> Result<()> {
    validate_claim(claim, bundle)?;
    let label = match claim.assessment {
        Assessment::Supported => "Supported",
        Assessment::Contested => "Contested",
        Assessment::Unresolved => "Unresolved",
        Assessment::Refuted => "REFUTED HYPOTHESIS / not asserted",
    };
    markdown.push_str(&format!(
        "\n- **{}{}** [{}]: {}",
        label,
        if claim.inference { ", inference" } else { "" },
        safe_text(&claim.claim_id),
        safe_text(&claim.text)
    ));
    for reference in &claim.evidence_refs {
        let source = bundle
            .sources
            .iter()
            .find(|s| s.source_id == reference.source_id)
            .ok_or("uncaptured report source")?;
        let locator = source.locator.replace('<', "%3C").replace('>', "%3E");
        markdown.push_str(&format!(
            " [{}:{}](<{}>)",
            source.source_id, source.excerpt.start_line, locator
        ));
    }
    markdown.push('\n');
    Ok(())
}

fn safe_text(value: &str) -> String {
    let printable = value
        .chars()
        .filter_map(|character| {
            if matches!(character, '\r' | '\n' | '\t') {
                Some(' ')
            } else if character.is_control() {
                None
            } else {
                Some(character)
            }
        })
        .collect::<String>();
    printable
        .replace('\\', "\\\\")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('`', "\\`")
}
fn normalize_quote(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn bounded_text(value: &str, max: usize, name: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > max || value.contains('\0') {
        return Err(format!("invalid or overlarge {name}").into());
    }
    Ok(())
}
fn bounded_optional_text(value: &str, max: usize, name: &str) -> Result<()> {
    if !value.is_empty() {
        bounded_text(value, max, name)?;
    }
    Ok(())
}
fn identifier(value: &str, max: usize) -> Result<()> {
    if value.is_empty()
        || value.len() > max
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':' | b'/'))
    {
        return Err("invalid research identifier".into());
    }
    Ok(())
}
fn digest_shape(value: &str) -> Result<()> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err("research digest lacks SHA-256 identity".into());
    };
    if hex.len() != 64
        || !hex
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err("invalid research SHA-256 identity".into());
    }
    Ok(())
}

pub fn stage_output_schema() -> Value {
    let text = |max| json!({"type":"string","maxLength":max});
    let assessment =
        json!({"type":"string","enum":["supported","contested","unresolved","refuted"]});
    let evidence = json!({"type":"object","additionalProperties":false,"required":["source_id","excerpt_id","quote","stance"],"properties":{"source_id":text(64),"excerpt_id":text(96),"quote":text(16384),"stance":{"type":"string","enum":["support","counter"]}}});
    let refs = json!({"type":"array","maxItems":24,"items":evidence});
    let claim = json!({"type":"object","additionalProperties":false,"required":["claim_id","text","assessment","inference","evidence_refs"],"properties":{"claim_id":text(192),"text":text(8192),"assessment":assessment,"inference":{"type":"boolean"},"evidence_refs":refs}});
    let challenge = json!({"type":"object","additionalProperties":false,"required":["claim_id","assessment","reason","evidence_refs"],"properties":{"claim_id":text(192),"assessment":assessment,"reason":text(4096),"evidence_refs":refs}});
    let section = json!({"type":"object","additionalProperties":false,"required":["heading","claim_ids"],"properties":{"heading":text(512),"claim_ids":{"type":"array","maxItems":64,"items":text(192)}}});
    json!({"type":"object","additionalProperties":false,"required":["agent_reported_advisory","claims","challenges","report_sections","unresolved"],"properties":{"agent_reported_advisory":text(4096),"claims":{"type":"array","maxItems":64,"items":claim},"challenges":{"type":"array","maxItems":64,"items":challenge},"report_sections":{"type":"array","maxItems":32,"items":section},"unresolved":{"type":"array","maxItems":64,"items":text(2048)}}})
}

#[cfg(test)]
mod tests;
