use super::*;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    schema_version: u8,
    cases: Vec<Case>,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    name: String,
    mutation: String,
    expected_valid: bool,
}

fn fixture() -> Result<(ResearchRequest, SourceBundle, StageArtifact)> {
    let text = "pub fn answer() -> u32 { 42 }\n";
    let mut request = ResearchRequest::new("What does answer return?", "/fixture".into());
    request.sources.push(SourceSpec::Repository {
        path: "src/lib.rs".into(),
    });
    let mut bundle = SourceBundle {
        sources: vec![SourceRecord {
            source_id: "s1".into(),
            kind: SourceKind::Repository,
            locator: "/fixture/src/lib.rs".into(),
            observed_at: "2026-11-05T00:00:00Z".into(),
            body_sha256: sha256(text.as_bytes()),
            body_bytes: text.len() as u64,
            media_type: "text/plain".into(),
            redirect_chain: vec![],
            excerpt: Excerpt {
                excerpt_id: "e1".into(),
                source_id: "s1".into(),
                text: text.into(),
                sha256: sha256(text.as_bytes()),
                start_byte: 0,
                end_byte: text.len() as u64,
                start_line: 1,
                end_line: 1,
            },
        }],
        failures: vec![],
        snapshot_digest: String::new(),
    };
    bundle.snapshot_digest = sha256(&serde_json::to_vec(&(&bundle.sources, &bundle.failures))?);
    let artifact = StageArtifact {
        claims: vec![ResearchClaim {
            claim_id: "c1".into(),
            text: "answer returns 42".into(),
            assessment: Assessment::Supported,
            inference: false,
            evidence_refs: vec![EvidenceRef {
                source_id: "s1".into(),
                excerpt_id: "e1".into(),
                quote: "42".into(),
                stance: EvidenceStance::Support,
            }],
        }],
        ..StageArtifact::default()
    };
    Ok((request, bundle, artifact))
}

fn exercise(mutation: &str) -> Result<()> {
    let (mut request, mut bundle, mut artifact) = fixture()?;
    match mutation {
        "valid_repository" => {}
        "valid_document" => {
            request.sources = vec![SourceSpec::TechnicalDocument {
                path: "guide.md".into(),
            }];
            bundle.sources[0].kind = SourceKind::TechnicalDocument;
            bundle.sources[0].locator = "/fixture/guide.md".into();
        }
        "valid_web" => {
            request.sources = vec![SourceSpec::Web {
                url: "https://example.com/guide".into(),
            }];
            bundle.sources[0].kind = SourceKind::Web;
            bundle.sources[0].locator = "https://example.com/guide".into();
        }
        "discovery_valid"
        | "discovery_events_unbounded"
        | "discovery_candidates_unbounded"
        | "discovery_minimum_unreachable"
        | "discovery_native_only"
        | "discovery_timeout_unbounded" => {
            let mut config = DiscoveryConfig::default_web();
            if let DiscoveryConfig::Web {
                model,
                max_search_events,
                max_candidates,
                min_accepted_sources,
                timeout_seconds,
                ..
            } = &mut config
            {
                match mutation {
                    "discovery_events_unbounded" => *max_search_events = MAX_SEARCH_EVENTS + 1,
                    "discovery_candidates_unbounded" => {
                        *max_candidates = MAX_DISCOVERY_CANDIDATES + 1
                    }
                    "discovery_minimum_unreachable" => *min_accepted_sources = *max_candidates + 1,
                    "discovery_native_only" => *model = "gemini-4-argon-high".into(),
                    "discovery_timeout_unbounded" => *timeout_seconds = 121,
                    _ => {}
                }
            }
            request.discovery = Some(config);
        }
        "discovery_unknown_field" => {
            let mut value = serde_json::to_value(request)?;
            value["discovery"] = json!({"kind":"web","new_authority":true});
            let _invalid: ResearchRequest = serde_json::from_value(value)?;
            return Ok(());
        }
        "unknown_source" => artifact.claims[0].evidence_refs[0].source_id = "invented".into(),
        "unknown_excerpt" => artifact.claims[0].evidence_refs[0].excerpt_id = "invented".into(),
        "duplicate_evidence_reference" => {
            let reference = artifact.claims[0].evidence_refs[0].clone();
            artifact.claims[0].evidence_refs.push(reference);
        }
        "tampered_excerpt" => bundle.sources[0].excerpt.text = "different bytes".into(),
        "bad_range" => bundle.sources[0].excerpt.end_byte += 1,
        "snapshot_mismatch" => bundle.snapshot_digest = sha256(b"different snapshot"),
        "credential_path" => {
            request.sources = vec![SourceSpec::Repository {
                path: ".env".into(),
            }]
        }
        "credential_query" => {
            request.sources = vec![SourceSpec::Web {
                url: "https://example.com/?access_token=private".into(),
            }]
        }
        "empty_challenger" => {
            return validate_stage(
                ResearchPhase::Challenge,
                &StageArtifact::default(),
                &bundle,
                &artifact.claims,
                &request.budget,
            );
        }
        "complete_challenge_coverage"
        | "missing_challenge"
        | "advisory_only_challenge"
        | "duplicate_challenge"
        | "foreign_challenge" => {
            let mut prior = artifact.claims.clone();
            let mut second = prior[0].clone();
            second.claim_id = "c2".into();
            prior.push(second);
            let mut review = StageArtifact {
                agent_reported_advisory: "Review is advisory; every claim is assessed below."
                    .into(),
                challenges: prior
                    .iter()
                    .map(|claim| ClaimChallenge {
                        claim_id: claim.claim_id.clone(),
                        assessment: claim.assessment,
                        reason: "Checked this claim against its captured support.".into(),
                        evidence_refs: vec![],
                    })
                    .collect(),
                ..StageArtifact::default()
            };
            match mutation {
                "missing_challenge" => {
                    review.challenges.pop();
                }
                "advisory_only_challenge" => review.challenges.clear(),
                "duplicate_challenge" => review.challenges.push(review.challenges[0].clone()),
                "foreign_challenge" => review.challenges[0].claim_id = "missing".into(),
                _ => {}
            }
            return validate_stage(
                ResearchPhase::Challenge,
                &review,
                &bundle,
                &prior,
                &request.budget,
            );
        }
        "absent_quote" => artifact.claims[0].evidence_refs[0].quote = "returns 99".into(),
        "unsupported_claim" => artifact.claims[0].evidence_refs.clear(),
        "refuted_no_counter" => artifact.claims[0].assessment = Assessment::Refuted,
        "duplicate_claim" => artifact.claims.push(artifact.claims[0].clone()),
        "duplicate_source" => {
            bundle.sources.push(bundle.sources[0].clone());
            request.sources.push(SourceSpec::Repository {
                path: "other.rs".into(),
            });
        }
        "source_outside" => bundle.sources[0].locator = "https://invented.example/".into(),
        "oversized_quote" => artifact.claims[0].evidence_refs[0].quote = "x".repeat(16_385),
        "write_input" | "unknown_field" => {
            let mut value = serde_json::to_value(request)?;
            value[if mutation == "write_input" {
                "write_files"
            } else {
                "new_authority"
            }] = json!(["src/lib.rs"]);
            let _invalid: ResearchRequest = serde_json::from_value(value)?;
            return Ok(());
        }
        "budget_overflow" => request.budget.max_parallel = 100,
        "valid_refuted_conclusion"
        | "refuted_missing_quote"
        | "refuted_without_conclusion"
        | "omitted_refuted_claim" => {
            let mut prior = artifact.claims.clone();
            prior[0].assessment = Assessment::Refuted;
            prior[0].text = "answer returns 99".into();
            prior[0].evidence_refs[0].stance = EvidenceStance::Counter;
            if mutation == "refuted_missing_quote" {
                prior[0].evidence_refs[0].quote.clear();
            }
            if mutation == "omitted_refuted_claim" {
                let mut second = prior[0].clone();
                second.claim_id = "c2".into();
                prior.push(second);
            }
            let sections = if mutation == "refuted_without_conclusion" {
                Vec::new()
            } else {
                vec![ReportSection {
                    heading: "Conclusion".into(),
                    claim_ids: vec!["c1".into()],
                }]
            };
            let synth = StageArtifact {
                report_sections: sections,
                unresolved: if mutation == "refuted_without_conclusion" {
                    vec!["No conclusion was rendered.".into()]
                } else {
                    Vec::new()
                },
                ..StageArtifact::default()
            };
            return validate_stage(
                ResearchPhase::Synthesize,
                &synth,
                &bundle,
                &prior,
                &request.budget,
            );
        }
        "synth_unknown" | "synth_refuted" | "synth_unresolved" | "omitted_supported_claim" => {
            let mut prior = artifact.claims.clone();
            if mutation == "omitted_supported_claim" {
                let mut second = prior[0].clone();
                second.claim_id = "c2".into();
                prior.push(second);
            }
            let id = if mutation == "synth_unknown" {
                "missing"
            } else {
                "c1"
            };
            if mutation == "synth_refuted" {
                prior[0].assessment = Assessment::Refuted;
            }
            if mutation == "synth_unresolved" {
                prior[0].assessment = Assessment::Unresolved;
            }
            let synth = StageArtifact {
                report_sections: vec![ReportSection {
                    heading: "Findings".into(),
                    claim_ids: vec![id.into()],
                }],
                ..StageArtifact::default()
            };
            return validate_stage(
                ResearchPhase::Synthesize,
                &synth,
                &bundle,
                &prior,
                &request.budget,
            );
        }
        "challenge_upgrade" => {
            let mut prior = artifact.claims.clone();
            prior[0].assessment = Assessment::Contested;
            let challenge = StageArtifact {
                challenges: vec![ClaimChallenge {
                    claim_id: "c1".into(),
                    assessment: Assessment::Supported,
                    reason: "I agree".into(),
                    evidence_refs: vec![],
                }],
                ..StageArtifact::default()
            };
            return validate_stage(
                ResearchPhase::Challenge,
                &challenge,
                &bundle,
                &prior,
                &request.budget,
            );
        }
        _ => return Err("unknown deterministic harness fixture".into()),
    }
    if matches!(mutation, "valid_document" | "valid_web") {
        bundle.snapshot_digest = sha256(&serde_json::to_vec(&(&bundle.sources, &bundle.failures))?);
    }
    validate_request(&request)?;
    validate_sources(&request, &bundle)?;
    validate_stage(
        ResearchPhase::Investigate,
        &artifact,
        &bundle,
        &[],
        &request.budget,
    )
}

#[test]
fn canonical_corpus_uses_the_runtime_validator() -> Result<()> {
    let corpus: Corpus = serde_json::from_str(include_str!(
        "../../../tests/fixtures/research_contract_cases.json"
    ))?;
    assert_eq!(corpus.schema_version, 1);
    for case in corpus.cases {
        let result = exercise(&case.mutation);
        assert_eq!(
            result.is_ok(),
            case.expected_valid,
            "{}: {result:?}",
            case.name
        );
    }
    Ok(())
}

#[test]
fn host_rendering_labels_refuted_conclusions_and_preserves_annex() -> Result<()> {
    let (request, bundle, artifact) = fixture()?;
    let mut claims = artifact.claims;
    claims[0].text = "answer returns 99 [forged](https://uncaptured.example)\u{1b}[31m".into();
    let challenge = StageArtifact {
        challenges: vec![ClaimChallenge {
            claim_id: "c1".into(),
            assessment: Assessment::Refuted,
            reason: "Counterexample".into(),
            evidence_refs: vec![EvidenceRef {
                source_id: "s1".into(),
                excerpt_id: "e1".into(),
                quote: "42".into(),
                stance: EvidenceStance::Counter,
            }],
        }],
        ..StageArtifact::default()
    };
    validate_stage(
        ResearchPhase::Challenge,
        &challenge,
        &bundle,
        &claims,
        &request.budget,
    )?;
    apply_challenges(&mut claims, &challenge)?;
    let conclusion = StageArtifact {
        report_sections: vec![ReportSection {
            heading: "Conclusion".into(),
            claim_ids: vec!["c1".into()],
        }],
        ..StageArtifact::default()
    };
    validate_stage(
        ResearchPhase::Synthesize,
        &conclusion,
        &bundle,
        &claims,
        &request.budget,
    )?;
    let report = render_report(&claims, &conclusion.report_sections, &[], &bundle)?;
    assert_eq!(
        report.matches("REFUTED HYPOTHESIS / not asserted").count(),
        2
    );
    assert!(!report.contains("**Supported"));
    assert!(report.contains("Unresolved, refuted and omitted findings"));
    assert!(report.contains("NOT_MECHANICALLY_PROVEN"));
    assert!(report.contains("\\[forged\\]"));
    assert!(!report.contains('\u{1b}'));
    claims[0]
        .evidence_refs
        .retain(|reference| reference.stance != EvidenceStance::Counter);
    assert!(render_report(&claims, &conclusion.report_sections, &[], &bundle).is_err());
    Ok(())
}

#[test]
fn stage_schema_is_closed_and_accepts_the_typed_positive_artifact() -> Result<()> {
    let (_, _, artifact) = fixture()?;
    crate::catalog::validate_schema(&serde_json::to_value(&artifact)?, &stage_output_schema())?;
    let mut bad = serde_json::to_value(artifact)?;
    bad["workspace_write"] = json!(true);
    assert!(crate::catalog::validate_schema(&bad, &stage_output_schema()).is_err());
    Ok(())
}
