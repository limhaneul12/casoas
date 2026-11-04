use super::*;
use std::fs;

const PYTHON_COMMANDS: &[&str] = &[
    "/help",
    "/ask",
    "/prompt",
    "/sonnet",
    "/opus",
    "/argon",
    "/flash",
    "/use",
    "/model",
    "/skills",
    "/skill",
    "/dag-mode",
    "/dag-exit",
    "/dag-ulw",
    "/dag",
    "/dag-run",
    "/effect",
    "/effort",
    "/reasoning",
    "/dag-effect",
    "/prompts",
    "/promt",
    "/trace",
    "/back",
    "/prev",
    "/compact",
    "/clear",
    "/diff",
    "/review",
    "/permissions",
    "/approvals",
    "/resume",
    "/triad",
    "/models",
    "/lanes",
    "/omh",
    "/board",
    "/council",
    "/votes",
    "/recovery",
    "/todo",
    "/plan",
    "/status",
    "/inbox",
    "/jobs",
    "/theme",
    "/quit",
    "/exit",
];

#[test]
fn git_branch_metadata_handles_normal_worktree_detached_and_missing_repositories() -> Result<()> {
    let root = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let no_git = root.path().join("plain");
    fs::create_dir(&no_git)?;
    assert_eq!(
        TuiCatalog::discover_with_home(&no_git, Some(home.path()))?.git_branch(),
        None
    );

    let repository = root.path().join("repository");
    fs::create_dir_all(repository.join(".git/worktrees/topic"))?;
    fs::create_dir_all(repository.join("src/nested"))?;
    fs::write(
        repository.join(".git/HEAD"),
        "ref: refs/heads/feature/rust-ui\n",
    )?;
    let catalog =
        TuiCatalog::discover_with_home(&repository.join("src/nested"), Some(home.path()))?;
    assert_eq!(catalog.git_branch(), Some("feature/rust-ui"));

    let worktree = root.path().join("worktree");
    fs::create_dir_all(worktree.join("src"))?;
    fs::write(
        worktree.join(".git"),
        "gitdir: ../repository/.git/worktrees/topic\n",
    )?;
    fs::write(
        repository.join(".git/worktrees/topic/HEAD"),
        "ref: refs/heads/topic/worktree\n",
    )?;
    assert_eq!(
        TuiCatalog::discover_with_home(&worktree.join("src"), Some(home.path()))?.git_branch(),
        Some("topic/worktree")
    );
    fs::write(
        repository.join(".git/worktrees/topic/HEAD"),
        "0123456789abcdef0123456789abcdef01234567\n",
    )?;
    assert_eq!(
        TuiCatalog::discover_with_home(&worktree, Some(home.path()))?.git_branch(),
        Some("detached@01234567")
    );
    fs::write(
        repository.join(".git/worktrees/topic/HEAD"),
        "invalid repository metadata\n",
    )?;
    assert_eq!(
        TuiCatalog::discover_with_home(&worktree, Some(home.path()))?.git_branch(),
        None
    );
    Ok(())
}

#[test]
fn fuzzy_completion_preserves_prefix_marker() {
    let mut catalog = TuiCatalog::command_catalog();
    catalog.entries.push(entry(
        CompletionKind::Skill,
        "$codex-heterarchy",
        "$codex-heterarchy",
        "SKILL",
        "tri-polar",
        "",
        false,
    ));
    catalog.entries.push(entry(
        CompletionKind::File,
        "@src/tui.rs",
        "@src/tui.rs",
        "FILE",
        "source",
        "",
        false,
    ));
    assert_eq!(catalog.complete("/du", 3, 8)[0].value, "/dag-ulw");
    assert_eq!(catalog.complete("$ch", 3, 8)[0].value, "$codex-heterarchy");
    assert_eq!(catalog.complete("@st", 3, 8)[0].value, "@src/tui.rs");
}

#[test]
fn completion_catalog_contains_frozen_python_command_surface() {
    for command in PYTHON_COMMANDS {
        assert!(COMMANDS.contains(command), "missing {command}");
    }
}

#[test]
fn historical_contextual_recipes_and_korean_keywords_are_ranked() {
    let catalog = TuiCatalog::command_catalog();
    for (input, expected) in [
        ("/use a", "/use argon"),
        ("/use o", "/use opus"),
        ("/dag-mode e", "/dag-mode exit"),
        ("/skills co", "/skills core"),
        ("/model gpt-6.1-s", "/model gpt-6.1-sol"),
        ("/permissions read", "/permissions read-only"),
        ("/reasoning style roy", "/reasoning style royal-prism"),
    ] {
        let items = catalog.complete(input, input.len(), 64);
        assert_eq!(items[0].value, expected, "{input}");
        assert_eq!(items[0].replace_start, 0);
        assert!(items[0].submit_on_enter);
    }
    assert!(
        catalog
            .complete("오퍼스", "오퍼스".len(), 64)
            .iter()
            .any(|item| item.value.contains("opus"))
    );
    assert!(
        catalog
            .complete("/", 1, 64)
            .iter()
            .all(|item| item.value.starts_with('/'))
    );
    assert_eq!(
        catalog.complete_with_state("", 0, 64, &[], true)[0].value,
        "/skills"
    );
    assert!(catalog.complete("ordinary prose", 14, 64).is_empty());
}

#[test]
fn registered_global_and_plugin_skills_are_active_first_without_phantom_fallbacks() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    for (directory, name) in [
        (".agents/skills/core-a", "core-a"),
        (
            ".codex/plugins/cache/remote/library/0.1/skills/helper",
            "helper",
        ),
    ] {
        let path = home.path().join(directory);
        fs::create_dir_all(&path)?;
        fs::write(
            path.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: useful registered skill\n---\nREAL_BODY"),
        )?;
    }
    let catalog = TuiCatalog::discover_with_home(workspace.path(), Some(home.path()))?;
    let items = catalog.complete_with_state("$", 1, 64, &["helper".to_owned()], false);
    assert_eq!(items[0].value, "$library:helper");
    assert!(items[0].badge.contains("ON"));
    assert!(items.iter().any(|item| item.value == "$core-a"));
    assert!(!items.iter().any(|item| item.value == "$codex-heterarchy"));
    assert!(
        catalog
            .complete("$library:helper $", 17, 64)
            .iter()
            .all(|item| !item.value.ends_with(":helper"))
    );
    assert_eq!(
        catalog.complete("/skill core-a", 13, 64)[0].value,
        "/skill core-a"
    );
    assert_eq!(
        catalog.complete_with_state("/skills co", 10, 64, &["core-a".to_owned()], false)[0].value,
        "/skills core"
    );
    Ok(())
}

#[test]
fn unicode_quoted_file_paths_ranges_and_cursor_boundaries_are_preserved() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    fs::create_dir(workspace.path().join("src"))?;
    fs::write(workspace.path().join("src/한 글.rs"), "source")?;
    fs::create_dir(workspace.path().join("target"))?;
    fs::write(workspace.path().join("target/ignored.rs"), "generated")?;
    let catalog = TuiCatalog::discover_with_home(workspace.path(), Some(home.path()))?;
    let input = "Inspect @\"src/한";
    let items = catalog.complete(input, input.len(), 64);
    assert_eq!(items[0].value, "@\"src/한 글.rs\"");
    assert_eq!(items[0].replace_start, "Inspect ".len());
    let ranged = "@\"src/한 글.rs\":L2-L5";
    assert_eq!(catalog.complete(ranged, ranged.len(), 64)[0].value, ranged);
    assert!(catalog.complete("@target", 7, 64).is_empty());
    assert!(catalog.complete("한", 1, 64).is_empty());
    Ok(())
}

#[test]
fn file_completion_quotes_parser_delimiters_and_round_trips_actual_references() -> Result<()> {
    let workspace = tempfile::tempdir()?;
    let home = tempfile::tempdir()?;
    let paths = [
        "src/[slug]/page.tsx",
        "src/a(b).rs",
        "src/{route}.rs",
        "src/<part>.rs",
        "src/a,b.rs",
        "src/a;b.rs",
        "src/a`b.rs",
        "src/a'b.rs",
        "src/a@b.rs",
        "src/a:2.rs",
        "src/terminal.",
    ];
    for path in paths {
        let absolute = workspace.path().join(path);
        fs::create_dir_all(absolute.parent().ok_or("fixture parent missing")?)?;
        fs::write(absolute, "first\nsecond\nthird\n")?;
    }
    let catalog = TuiCatalog::discover_with_home(workspace.path(), Some(home.path()))?;
    let builder = crate::context::SourceContextBuilder::new(workspace.path())?;
    for path in paths {
        let quoted = format!("@{}", serde_json::to_string(path)?);
        let candidate = catalog
            .complete(&quoted, quoted.len(), 64)
            .into_iter()
            .find(|item| item.value == quoted)
            .ok_or("quoted file candidate missing")?;
        assert_eq!(
            builder.referenced_paths(&candidate.value)?,
            [std::path::PathBuf::from(path)]
        );
    }
    let ranged = "@\"src/[slug]/page.tsx\":L2-L3";
    let candidate = catalog
        .complete(ranged, ranged.len(), 64)
        .into_iter()
        .find(|item| item.value == ranged)
        .ok_or("quoted range candidate missing")?;
    let context = builder.build_with_text(&[], &[], &candidate.value, 4000)?;
    assert!(context.markdown.contains("second"));
    assert!(context.markdown.contains("third"));
    assert!(!context.markdown.contains("first"));
    let typed_file = "inspect page.tsx";
    let items = catalog.complete(typed_file, typed_file.len(), 64);
    assert!(
        items.iter().any(|item| item.value.contains("page.tsx")),
        "live typing of file basename should suggest @file completion"
    );
    Ok(())
}
