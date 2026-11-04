use super::*;

fn record() -> RecoveryRecord {
    RecoveryRecord {
        change_id: "change-1".into(),
        workspace: PathBuf::from("/workspace"),
        relative_path: PathBuf::from("src/main.rs"),
        before_sha256: digest(b"before"),
        after_sha256: digest(b"after"),
        before_len: 6,
        after_len: 5,
    }
}

#[test]
fn immutable_intent_reopens_and_conflicting_retry_preserves_original() -> crate::Result<()> {
    let temp = tempfile::tempdir()?;
    let original = record();
    {
        let store = Store::open(temp.path())?;
        assert_eq!(store.register_recovery(&original)?, original);
        assert_eq!(store.register_recovery(&original)?, original);
        let mut changed = original.clone();
        changed.after_sha256 = digest(b"other");
        assert!(store.register_recovery(&changed).is_err());
    }
    let reopened = Store::open(temp.path())?;
    assert_eq!(reopened.get_recovery(&original.change_id)?, Some(original));
    assert_eq!(reopened.get_recovery("absent")?, None);
    Ok(())
}

#[test]
fn concurrent_connections_choose_one_immutable_intent() -> crate::Result<()> {
    let temp = tempfile::tempdir()?;
    let a = Store::open(temp.path())?;
    let b = Store::open(temp.path())?;
    let first = record();
    let mut second = first.clone();
    second.after_sha256 = digest(b"different");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let other = barrier.clone();
    let left = std::thread::spawn(move || {
        barrier.wait();
        a.register_recovery(&first)
    });
    let right = std::thread::spawn(move || {
        other.wait();
        b.register_recovery(&second)
    });
    let l = left.join().map_err(|_| "left thread panicked")?;
    let r = right.join().map_err(|_| "right thread panicked")?;
    assert_ne!(l.is_ok(), r.is_ok());
    let winner = l.or(r)?;
    assert_eq!(
        Store::open(temp.path())?.get_recovery("change-1")?,
        Some(winner)
    );
    Ok(())
}

#[test]
fn corrupt_body_is_rejected_and_invalid_inputs_never_register() -> crate::Result<()> {
    let temp = tempfile::tempdir()?;
    let store = Store::open(temp.path())?;
    let original = record();
    for path in [
        "../escape",
        ".git/config",
        ".hcodex/changes/x",
        "/absolute",
        "a\\b",
        "",
    ] {
        let mut invalid = original.clone();
        invalid.relative_path = path.into();
        assert!(store.register_recovery(&invalid).is_err());
    }
    assert!(store.get_recovery("../bad").is_err());
    store.register_recovery(&original)?;
    lock(&store.jobs)?.execute(
        store.recovery_sql.query("corrupt_recovery_test", 2)?,
        params!["{}", original.change_id],
    )?;
    assert!(store.get_recovery("change-1").is_err());
    assert!(store.register_recovery(&original).is_err());
    Ok(())
}
