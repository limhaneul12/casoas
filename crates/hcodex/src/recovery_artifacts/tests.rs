use super::*;

#[test]
fn replay_reopen_and_conflict_preserve_bytes() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let archive = ChangeArchive::open(root.path())?;
    assert!(!root.path().join(".hcodex").exists());
    archive.prepare("change-1", b"old", b"new")?;
    archive.prepare("change-1", b"old", b"new")?;
    assert!(archive.prepare("change-1", b"different", b"new").is_err());
    assert_eq!(
        ChangeArchive::open(root.path())?.read("change-1")?,
        (b"old".to_vec(), b"new".to_vec())
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(root.path().join(".hcodex/changes/change-1/before"))?
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    Ok(())
}

#[test]
fn missing_read_and_invalid_input_create_nothing() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let archive = ChangeArchive::open(root.path())?;
    assert!(archive.read("missing").is_err());
    for id in ["", ".", "../escape", "a/b", "a\\b"] {
        assert!(archive.prepare(id, b"a", b"b").is_err());
    }
    assert!(archive.prepare(&"a".repeat(97), b"a", b"b").is_err());
    assert!(
        archive
            .prepare("oversized", &vec![0; MAX_BYTES + 1], b"b")
            .is_err()
    );
    assert!(!root.path().join(".hcodex").exists());
    Ok(())
}

#[test]
fn partial_publication_is_visible_and_never_repaired() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let archive = ChangeArchive::open(root.path())?;
    archive.prepare("partial", b"a", b"b")?;
    let after = root.path().join(".hcodex/changes/partial/after");
    fs::remove_file(&after)?;
    assert!(archive.read("partial").is_err());
    assert!(archive.prepare("partial", b"a", b"b").is_err());
    assert!(!after.exists());
    assert_eq!(
        fs::read(root.path().join(".hcodex/changes/partial/before"))?,
        b"a"
    );
    Ok(())
}

#[test]
fn read_limits_and_root_replacement_fail_closed() -> crate::Result<()> {
    let parent = tempfile::tempdir()?;
    let root = parent.path().join("workspace");
    fs::create_dir(&root)?;
    let archive = ChangeArchive::open(&root)?;
    archive.prepare("bounded", b"", &vec![1; MAX_BYTES])?;
    assert_eq!(archive.read("bounded")?.1.len(), MAX_BYTES);
    fs::write(
        root.join(".hcodex/changes/bounded/after"),
        vec![1; MAX_BYTES + 1],
    )?;
    assert!(archive.read("bounded").is_err());
    fs::rename(&root, parent.path().join("old"))?;
    fs::create_dir(&root)?;
    assert!(archive.prepare("new", b"a", b"b").is_err());
    assert!(!root.join(".hcodex").exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn static_symlinks_hardlinks_and_nonregular_files_are_rejected() -> crate::Result<()> {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    symlink(outside.path(), root.path().join(".hcodex"))?;
    let archive = ChangeArchive::open(root.path())?;
    assert!(archive.prepare("escape", b"a", b"b").is_err());
    assert_eq!(fs::read_dir(outside.path())?.count(), 0);
    fs::remove_file(root.path().join(".hcodex"))?;
    archive.prepare("links", b"a", b"b")?;
    let before = root.path().join(".hcodex/changes/links/before");
    fs::hard_link(&before, outside.path().join("alias"))?;
    assert!(archive.read("links").is_err());
    fs::remove_file(outside.path().join("alias"))?;
    fs::remove_file(&before)?;
    symlink(outside.path().join("unknown"), &before)?;
    assert!(archive.read("links").is_err());
    fs::remove_file(&before)?;
    fs::create_dir(&before)?;
    assert!(archive.read("links").is_err());
    Ok(())
}
