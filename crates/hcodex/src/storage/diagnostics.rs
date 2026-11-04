//! One-shot observational reads; deliberately never initializes or reconciles Store.

use std::fs;
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use sha2::{Digest, Sha256};
use url::Url;

use super::{JobRecord, SqlCatalog};

const MAX_RECORD_BYTES: i64 = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub(crate) enum ReadIssue {
    MissingDatabase,
    DatabaseUnavailable,
    InvalidDatabase,
    RecordTooLarge,
    InvalidRecord,
}

impl ReadIssue {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::MissingDatabase => "missing_database",
            Self::DatabaseUnavailable => "database_unavailable",
            Self::InvalidDatabase => "invalid_database",
            Self::RecordTooLarge => "record_too_large",
            Self::InvalidRecord => "invalid_record",
        }
    }
}

pub(crate) struct Snapshot {
    pub job: Option<JobRecord>,
    pub export: Option<JobRecord>,
    pub fingerprint: String,
}

pub(crate) enum ReadObservation {
    Observed(Box<Snapshot>),
    Unavailable(ReadIssue),
    Unknown(ReadIssue),
}

/// SQL reader locks may coordinate through SHM. This makes no domain writes and
/// does not promise byte-identical SQLite sidecars under concurrent writers.
pub(crate) fn read(root: &Path, job_id: &str) -> ReadObservation {
    match read_snapshot(root, job_id) {
        Ok(snapshot) => ReadObservation::Observed(Box::new(snapshot)),
        Err(ReadIssue::MissingDatabase) => ReadObservation::Unavailable(ReadIssue::MissingDatabase),
        Err(issue) => ReadObservation::Unknown(issue),
    }
}

fn read_snapshot(root: &Path, job_id: &str) -> Result<Snapshot, ReadIssue> {
    let root = root
        .canonicalize()
        .map_err(|_| ReadIssue::MissingDatabase)?;
    let jobs = root.join("jobs");
    if !real_file_kind(&jobs, true) {
        return Err(ReadIssue::MissingDatabase);
    }
    let path = jobs.join("jobs.db");
    if !real_file_kind(&path, false) {
        return Err(ReadIssue::MissingDatabase);
    }
    validate_existing_sidecars(&path)?;
    let before = fs::symlink_metadata(&path).map_err(|_| ReadIssue::DatabaseUnavailable)?;
    let mut uri = Url::from_file_path(&path).map_err(|()| ReadIssue::DatabaseUnavailable)?;
    uri.set_query(Some("mode=ro&cache=private"));
    let mut connection = Connection::open_with_flags(
        uri.as_str(),
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| ReadIssue::DatabaseUnavailable)?;
    connection
        .busy_timeout(Duration::from_millis(200))
        .map_err(|_| ReadIssue::DatabaseUnavailable)?;
    let catalog = SqlCatalog::parse(include_str!("../../sql/diagnostic_store.sql"))
        .map_err(|_| ReadIssue::InvalidDatabase)?;
    for name in ["pragma_query_only", "pragma_trusted_schema"] {
        connection
            .execute_batch(
                catalog
                    .query(name, 0)
                    .map_err(|_| ReadIssue::InvalidDatabase)?,
            )
            .map_err(|_| ReadIssue::DatabaseUnavailable)?;
    }
    let transaction = connection
        .transaction()
        .map_err(|_| ReadIssue::DatabaseUnavailable)?;
    let schema: i64 = transaction
        .query_row(
            catalog
                .query("pragma_schema_version", 0)
                .map_err(|_| ReadIssue::InvalidDatabase)?,
            [],
            |row| row.get(0),
        )
        .map_err(|_| ReadIssue::InvalidDatabase)?;
    let query = catalog
        .query("select_bounded_job", 2)
        .map_err(|_| ReadIssue::InvalidDatabase)?;
    let root_raw = read_row(&transaction, query, job_id)?;
    let export_id = format!("ckp-{job_id}-worktree-export");
    let export_raw = read_row(&transaction, query, &export_id)?;
    let job = decode(root_raw.as_deref(), job_id)?;
    let export = decode(export_raw.as_deref(), &export_id)?;
    let mut fingerprint = Sha256::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        fingerprint.update(before.dev().to_le_bytes());
        fingerprint.update(before.ino().to_le_bytes());
    }
    fingerprint.update(schema.to_le_bytes());
    for raw in [&root_raw, &export_raw] {
        match raw {
            Some(body) => {
                fingerprint.update([1]);
                fingerprint.update(
                    u64::try_from(body.len())
                        .map_err(|_| ReadIssue::RecordTooLarge)?
                        .to_le_bytes(),
                );
                fingerprint.update(body.as_bytes());
            }
            None => fingerprint.update([0]),
        }
    }
    let fingerprint = format!("sha256:{:x}", fingerprint.finalize());
    transaction
        .commit()
        .map_err(|_| ReadIssue::DatabaseUnavailable)?;
    drop(connection);
    validate_existing_sidecars(&path)?;
    let after = fs::symlink_metadata(&path).map_err(|_| ReadIssue::DatabaseUnavailable)?;
    if !same_identity(&before, &after) || !real_file_kind(&jobs, true) {
        return Err(ReadIssue::DatabaseUnavailable);
    }
    Ok(Snapshot {
        job,
        export,
        fingerprint,
    })
}

fn read_row(connection: &Connection, query: &str, id: &str) -> Result<Option<String>, ReadIssue> {
    let row: Option<(Option<String>, i64)> = connection
        .query_row(query, params![MAX_RECORD_BYTES, id], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .optional()
        .map_err(|_| ReadIssue::InvalidDatabase)?;
    match row {
        None => Ok(None),
        Some((Some(raw), size)) if (0..=MAX_RECORD_BYTES).contains(&size) => Ok(Some(raw)),
        Some(_) => Err(ReadIssue::RecordTooLarge),
    }
}

fn decode(raw: Option<&str>, id: &str) -> Result<Option<JobRecord>, ReadIssue> {
    raw.map(|body| {
        let record: JobRecord = serde_json::from_str(body).map_err(|_| ReadIssue::InvalidRecord)?;
        if record.job_id != id {
            return Err(ReadIssue::InvalidRecord);
        }
        Ok(record)
    })
    .transpose()
}

fn real_file_kind(path: &Path, directory: bool) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| {
        !metadata.file_type().is_symlink()
            && if directory {
                metadata.is_dir()
            } else {
                metadata.is_file()
            }
    })
}

fn same_identity(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        !right.file_type().is_symlink() && left.dev() == right.dev() && left.ino() == right.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (left, right);
        false
    }
}

fn validate_existing_sidecars(path: &Path) -> Result<(), ReadIssue> {
    for suffix in ["-wal", "-shm"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        match fs::symlink_metadata(Path::new(&name)) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(ReadIssue::DatabaseUnavailable),
        }
    }
    Ok(())
}
