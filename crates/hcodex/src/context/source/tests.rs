use std::fs;

use tempfile::tempdir;

use super::*;

#[test]
fn quoted_unicode_mentions_and_ranges_are_included_with_other_files() -> Result<()> {
    let workspace = tempdir()?;
    fs::create_dir(workspace.path().join("한글 code"))?;
    fs::write(
        workspace.path().join("한글 code/긴 함수.rs"),
        (1..=300)
            .map(|line| format!("actual line {line}\n"))
            .collect::<String>(),
    )?;
    fs::write(workspace.path().join("other.txt"), "OTHER_FILE_BODY")?;
    fs::write(
        workspace.path().join("literal:12.txt"),
        "COLON_FILENAME_BODY",
    )?;
    let builder = SourceContextBuilder::new(workspace.path())?;
    let text = "Inspect @\"한글 code/긴 함수.rs\":L250-L252";
    let context = builder.build_with_text(&[PathBuf::from("other.txt")], &[], text, 16_000)?;
    assert!(context.markdown.contains("250 | actual line 250"));
    assert!(!context.markdown.contains("249 | actual line 249"));
    assert!(context.markdown.contains("OTHER_FILE_BODY"));
    assert_eq!(context.excerpts[0].start_line, 250);
    assert_eq!(context.excerpts[0].end_line, 252);
    let marker = builder.build_with_text(
        &[],
        &[],
        "=== ACTIVE REGISTERED SKILLS is literal user text\nInspect @\"한글 code/긴 함수.rs\":L250-L252",
        16_000,
    )?;
    assert_eq!(marker.excerpts[0].start_line, 250);
    assert_eq!(marker.excerpts[0].end_line, 252);
    assert!(
        builder
            .build_with_text(&[], &[], "@\"literal:12.txt\"", 16_000)?
            .markdown
            .contains("COLON_FILENAME_BODY")
    );
    assert_eq!(
        builder.referenced_paths(text)?,
        [PathBuf::from("한글 code/긴 함수.rs")]
    );
    for invalid in [
        "@missing",
        "@../secret",
        "@/etc/passwd",
        "@other.txt:0",
        "@other.txt:4-2",
        "Inspect @\"other.txt",
        "Inspect @other.txt and @\"unfinished",
    ] {
        assert!(
            builder.build_with_text(&[], &[], invalid, 16_000).is_err(),
            "{invalid}"
        );
    }
    Ok(())
}

#[test]
fn source_budget_counts_headers_and_preserves_utf8_and_late_definitions() -> Result<()> {
    let workspace = tempdir()?;
    let mut source = "// prefix 한글\n".repeat(230);
    for index in 0..25 {
        source.push_str(&format!("fn earlier_{index}() {{}}\n"));
    }
    source.push_str("fn late_definition() {\n");
    source.push_str(&"    // actual inner body 한글\n".repeat(40));
    source.push_str("    // ACTUAL_LATE_BODY\n}\n");
    fs::write(workspace.path().join("late.rs"), source)?;
    let builder = SourceContextBuilder::new(workspace.path())?;
    let complete = builder.build_with_text(
        &[PathBuf::from("late.rs")],
        &[],
        "Inspect late_definition",
        16_000,
    )?;
    assert!(complete.markdown.contains("ACTUAL_LATE_BODY"));
    assert!(complete.excerpts[0].start_line > 200);
    for budget in [0, 16, 32, 100, 255, 1024] {
        let limited = builder.build_with_text(
            &[PathBuf::from("late.rs")],
            &[],
            "Inspect late_definition",
            budget,
        )?;
        assert!(limited.markdown.len() <= budget);
        assert!(limited.truncated);
    }
    assert!(SourceContextBuilder::new(&workspace.path().join("late.rs")).is_err());
    Ok(())
}

#[test]
fn indexed_file_and_diff_context_include_complete_late_symbol_range() -> Result<()> {
    let workspace = tempdir()?;
    fs::create_dir(workspace.path().join(".codegraph"))?;
    let database = workspace.path().join(".codegraph/codegraph.db");
    let writer = rusqlite::Connection::open(database)?;
    writer.execute_batch("CREATE TABLE nodes(id TEXT,kind TEXT,name TEXT,qualified_name TEXT,file_path TEXT,start_line INTEGER,end_line INTEGER); CREATE TABLE edges(source TEXT,target TEXT); INSERT INTO nodes VALUES('function:chosen','function','chosen','pkg.chosen','large.rs',220,280);")?;
    drop(writer);
    let source = (1..=320)
        .map(|line| match line {
            220 => "fn chosen() {\n".to_owned(),
            260 => "    // INDEXED_BODY_END_MARKER\n".to_owned(),
            280 => "}\n".to_owned(),
            _ => format!("// actual {line}\n"),
        })
        .collect::<String>();
    fs::write(workspace.path().join("large.rs"), source)?;
    let builder = SourceContextBuilder::new(workspace.path())?;
    for text in [
        "",
        "--- a/large.rs\n+++ b/large.rs\n@@ -250,2 +250,2 @@\n-old\n+new",
    ] {
        let context = builder.build_with_text(&[PathBuf::from("large.rs")], &[], text, 16_000)?;
        assert!(context.markdown.contains("INDEXED_BODY_END_MARKER"));
        assert_eq!(context.excerpts[0].start_line, 220);
        assert_eq!(context.excerpts[0].end_line, 280);
    }
    let new_file = builder.build_with_text(
        &[],
        &[],
        "--- /dev/null\n+++ b/new.rs\n@@ -0,0 +1 @@\n+fn added() {}",
        16_000,
    )?;
    assert!(new_file.markdown.contains("not present on disk"));
    Ok(())
}

#[test]
fn injects_real_numbered_source_and_rejects_escape() -> Result<()> {
    let workspace = tempdir()?;
    fs::write(
        workspace.path().join("module.rs"),
        "fn first() {}\nfn selected_symbol() {}\nfn last() {}\n",
    )?;
    let builder = SourceContextBuilder::new(workspace.path())?;
    let context = builder.build(
        &[PathBuf::from("module.rs")],
        &["selected_symbol".to_owned()],
        16_000,
    )?;
    assert!(context.markdown.contains("2 | fn selected_symbol()"));
    assert!(
        builder
            .build(&[PathBuf::from("../secret")], &[], 100)
            .is_err()
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn rejects_symlink_that_escapes_workspace() -> Result<()> {
    use std::os::unix::fs::symlink;

    let workspace = tempdir()?;
    let outside = tempdir()?;
    fs::write(outside.path().join("secret"), "secret")?;
    symlink(outside.path().join("secret"), workspace.path().join("link"))?;
    let builder = SourceContextBuilder::new(workspace.path())?;
    assert!(builder.build(&[PathBuf::from("link")], &[], 100).is_err());
    Ok(())
}
