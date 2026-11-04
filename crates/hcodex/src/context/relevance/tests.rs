use super::*;

#[test]
fn ranks_explicit_path_and_content_deterministically() -> crate::Result<()> {
    let workspace = tempfile::tempdir()?;
    fs::create_dir(workspace.path().join("src"))?;
    fs::write(
        workspace.path().join("src/router.rs"),
        "fn route_request() {}",
    )?;
    fs::write(workspace.path().join("src/store.rs"), "fn persist_job() {}")?;
    let result = discover_relevance(
        workspace.path(),
        "fix route_request in @src/router.rs and inspect persistence",
        2,
    )?;
    assert_eq!(result.files[0], PathBuf::from("src/router.rs"));
    assert!(result.candidates[0].score >= 1_000);
    assert_eq!(
        result.selection_reason,
        "ranked explicit paths, CodeGraph symbols, path tokens, and bounded source matches"
    );
    Ok(())
}

#[test]
fn declares_inventory_fallback_when_prompt_has_no_match() -> crate::Result<()> {
    let workspace = tempfile::tempdir()?;
    fs::write(workspace.path().join("a.rs"), "fn alpha() {}")?;
    fs::write(workspace.path().join("b.rs"), "fn beta() {}")?;
    let result = discover_relevance(workspace.path(), "무관한 요청", 1)?;
    assert_eq!(result.files, [PathBuf::from("a.rs")]);
    assert_eq!(
        result.selection_reason,
        "workspace_inventory_fallback_no_semantic_match"
    );
    assert!(result.candidates[0].rationale[0].contains("fallback"));
    Ok(())
}

#[test]
fn korean_domain_terms_expand_and_generic_prompts_stay_unexpanded() -> crate::Result<()> {
    let tokens = query_tokens("정산 후 자동완성과 렌더링 라우팅 기능 구현")?;
    let verifier_pos = tokens
        .iter()
        .position(|t| t == "verifier")
        .ok_or("missing verifier")?;
    let completion_pos = tokens
        .iter()
        .position(|t| t == "completion")
        .ok_or("missing completion")?;
    let render_pos = tokens
        .iter()
        .position(|t| t == "render")
        .ok_or("missing render")?;
    let routing_pos = tokens
        .iter()
        .position(|t| t == "routing")
        .ok_or("missing routing")?;
    assert!(verifier_pos < completion_pos);
    assert!(completion_pos < render_pos);
    assert!(render_pos < routing_pos);
    assert_eq!(query_tokens("무관한 요청")?, vec!["무관한".to_owned()]);
    Ok(())
}
