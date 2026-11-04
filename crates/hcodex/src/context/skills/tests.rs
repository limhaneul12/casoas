use std::fs;
use std::path::{Path, PathBuf};

use tempfile::tempdir;

use super::*;

fn write_skill(root: &Path, name: &str, body: &str) -> Result<PathBuf> {
    let directory = root.join(name);
    fs::create_dir_all(&directory)?;
    let path = directory.join("SKILL.md");
    fs::write(&path, format!("---\nname: {name}\n---\n{body}"))?;
    Ok(path)
}

#[test]
fn core_preset_replaces_active_skills_and_reads_all_real_bodies() -> Result<()> {
    let workspace = tempdir()?;
    let home = tempdir()?;
    write_skill(
        &home.path().join(".codex/skills"),
        DEFAULT_SKILL,
        "REAL_HETERARCHY",
    )?;
    write_skill(
        &home.path().join(".codex/skills"),
        "codegraph-operator",
        "GLOBAL_GRAPH",
    )?;
    let local_graph = write_skill(
        &workspace.path().join(".codex/skills"),
        "codegraph-operator",
        "LOCAL_GRAPH",
    )?;
    write_skill(
        &home.path().join(".agents/skills"),
        "codebase-refactor-guardian",
        "REAL_GUARDIAN",
    )?;
    write_skill(&home.path().join(".codex/skills"), "custom", "EXTRA")?;
    let mut registry = SkillRegistry::discover(workspace.path(), Some(home.path()))?;
    registry.activate("custom")?;
    assert!(registry.activate_core().is_empty());
    assert_eq!(registry.active(), CORE_SKILLS);
    assert!(registry.activate_core().is_empty());
    assert_eq!(registry.active(), CORE_SKILLS);
    assert_eq!(
        registry.skills()["codegraph-operator"].path,
        local_graph.canonicalize()?
    );
    let context = registry.active_context(32_000)?;
    for body in ["REAL_HETERARCHY", "LOCAL_GRAPH", "REAL_GUARDIAN"] {
        assert!(context.contains(body));
    }
    assert!(!context.contains("GLOBAL_GRAPH"));
    assert!(!context.contains("EXTRA"));
    let previous = registry.active().to_vec();
    assert!(registry.activate("unknown").is_err());
    assert_eq!(registry.active(), previous);
    assert!(registry.active_context(1)?.contains("truncated at 1"));
    Ok(())
}

#[test]
fn missing_core_members_are_reported_without_phantom_activation() -> Result<()> {
    let workspace = tempdir()?;
    let home = tempdir()?;
    let mut registry = SkillRegistry::discover(workspace.path(), Some(home.path()))?;
    assert!(registry.active().is_empty());
    assert_eq!(registry.activate_core(), CORE_SKILLS);
    assert!(registry.active().is_empty());
    Ok(())
}

#[cfg(unix)]
#[test]
fn linked_user_skills_are_discovered_without_following_cycles_forever() -> Result<()> {
    let workspace = tempdir()?;
    let home = tempdir()?;
    let package = tempdir()?;
    write_skill(package.path(), "linked", "LINKED_REAL_BODY")?;
    let root = home.path().join(".agents/skills");
    fs::create_dir_all(&root)?;
    std::os::unix::fs::symlink(package.path().join("linked"), root.join("linked"))?;
    std::os::unix::fs::symlink(&root, root.join("cycle"))?;
    let mut registry = SkillRegistry::discover(workspace.path(), Some(home.path()))?;
    registry.activate("linked")?;
    assert!(
        registry
            .active_context(32_000)?
            .contains("LINKED_REAL_BODY")
    );
    Ok(())
}

#[test]
fn discovers_reads_and_activates_real_skill_bodies() -> Result<()> {
    let workspace = tempdir()?;
    let home = tempdir()?;
    let skill_dir = home.path().join(".codex/skills/codex-heterarchy");
    fs::create_dir_all(&skill_dir)?;
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: codex-heterarchy\ndescription: Tri-polar routing\n---\nREAL_SKILL_BODY",
    )?;
    let mut registry = SkillRegistry::discover(workspace.path(), Some(home.path()))?;
    assert_eq!(registry.active(), &["codex-heterarchy"]);
    assert!(registry.active_context(16_000)?.contains("REAL_SKILL_BODY"));
    let (matched, remaining) = registry.extract_mentions("$codex-heterarchy audit this");
    assert_eq!(matched, ["codex-heterarchy"]);
    assert_eq!(remaining, "audit this");
    Ok(())
}

#[test]
fn folded_multiline_frontmatter_and_workspace_skills_root_are_discovered() -> Result<()> {
    let workspace = tempdir()?;
    let home = tempdir()?;
    let skill_dir = workspace.path().join("skills/codebase-refactor-guardian");
    fs::create_dir_all(&skill_dir)?;
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: codebase-refactor-guardian\ndescription: >-\n  Guardian skill for refactoring\n  and harness compliance.\n---\n# Guardian\nBody",
    )?;
    let registry = SkillRegistry::discover(workspace.path(), Some(home.path()))?;
    let entry = &registry.skills()["codebase-refactor-guardian"];
    assert_eq!(entry.scope, SkillScope::Workspace);
    assert_eq!(
        entry.description,
        "Guardian skill for refactoring and harness compliance."
    );
    Ok(())
}
