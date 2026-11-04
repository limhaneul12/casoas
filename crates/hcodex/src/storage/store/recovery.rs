//! Immutable file-change intent metadata, owned by the existing jobs connection.
use std::path::{Component, PathBuf};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{SqlCatalog, Store, lock};

/// Input representation. Registration and every read validate its invariants.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecoveryRecord {
    pub change_id: String,
    pub workspace: PathBuf,
    pub relative_path: PathBuf,
    pub before_sha256: String,
    pub after_sha256: String,
    pub before_len: u64,
    pub after_len: u64,
}

impl RecoveryRecord {
    pub fn validate(&self) -> crate::Result<()> {
        validate_id(&self.change_id)?;
        if !self.workspace.is_absolute()
            || self
                .workspace
                .components()
                .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
            || self
                .workspace
                .to_str()
                .is_none_or(|path| path.contains('\0'))
        {
            return Err("invalid recovery workspace".into());
        }
        let text = self
            .relative_path
            .to_str()
            .ok_or("non-UTF8 recovery path")?;
        if text.is_empty()
            || text.contains(['\0', '\\'])
            || self.relative_path.components().any(|part| match part {
                Component::Normal(name) => name == ".git" || name == ".hcodex",
                _ => true,
            })
        {
            return Err("invalid recovery relative path".into());
        }
        for hash in [&self.before_sha256, &self.after_sha256] {
            let valid = hash.strip_prefix("sha256:").is_some_and(|hex| {
                hex.len() == 64
                    && hex
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            });
            if !valid {
                return Err("invalid recovery digest".into());
            }
        }
        if self.before_len > 1024 * 1024
            || self.after_len > 1024 * 1024
            || self.before_sha256 == self.after_sha256
        {
            return Err("invalid recovery content bounds or unchanged content".into());
        }
        Ok(())
    }
}

fn validate_id(id: &str) -> crate::Result<()> {
    if id.is_empty()
        || id.len() > 96
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err("invalid recovery change ID".into());
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn read(
    connection: &Connection,
    sql: &SqlCatalog,
    id: &str,
) -> crate::Result<Option<RecoveryRecord>> {
    let row: Option<(String, String, String)> = connection
        .query_row(sql.query("select_recovery", 1)?, [id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .optional()?;
    row.map(|(scalar, body, checksum)| {
        if body.len() > 16 * 1024 || digest(body.as_bytes()) != checksum {
            return Err("corrupt recovery record checksum".into());
        }
        let record: RecoveryRecord = serde_json::from_str(&body)?;
        record.validate()?;
        if scalar != id || record.change_id != scalar {
            return Err("corrupt recovery identity".into());
        }
        Ok(record)
    })
    .transpose()
}

impl Store {
    pub fn register_recovery(&self, record: &RecoveryRecord) -> crate::Result<RecoveryRecord> {
        record.validate()?;
        let body = serde_json::to_string(record)?;
        if body.len() > 16 * 1024 {
            return Err("recovery record exceeds metadata budget".into());
        }
        let mut connection = lock(&self.jobs)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = read(&transaction, &self.recovery_sql, &record.change_id)? {
            if existing != *record {
                return Err("recovery change ID conflicts with immutable intent".into());
            }
            transaction.commit()?;
            return Ok(existing);
        }
        transaction.execute(
            self.recovery_sql.query("insert_recovery", 3)?,
            params![record.change_id, body, digest(body.as_bytes())],
        )?;
        transaction.commit()?;
        Ok(record.clone())
    }

    pub fn get_recovery(&self, change_id: &str) -> crate::Result<Option<RecoveryRecord>> {
        validate_id(change_id)?;
        let connection = lock(&self.jobs)?;
        read(&connection, &self.recovery_sql, change_id)
    }
}

#[cfg(test)]
mod tests;
