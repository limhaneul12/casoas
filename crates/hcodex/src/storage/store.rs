use std::collections::BTreeMap;
use std::fs;
use std::io::{Error as IoError, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, ErrorCode, OptionalExtension, TransactionBehavior, params};
use serde_json::Value;
use uuid::Uuid;

use super::domain::{
    AuditVerdict, HandoffFilter, HandoffStatus, HandoffTicket, JobRecord, JobRegistration,
    JobStatus, JobTelemetry, NewHandoff, NewJob, OutcomeState, TriadRole,
};
use super::sql::{SqlCatalog, validate_text};

mod recovery;
pub use recovery::RecoveryRecord;

const DEFAULT_MAX_RETAINED: usize = 1_000;
const STALE_RUNNING_SECONDS: f64 = 3_600.0;

pub struct Store {
    jobs: Mutex<Connection>,
    handoffs: Mutex<Connection>,
    job_sql: SqlCatalog,
    handoff_sql: SqlCatalog,
    recovery_sql: SqlCatalog,
    max_retained_jobs: usize,
    max_retained_handoffs: usize,
}

impl Store {
    pub fn open(root: &Path) -> crate::Result<Self> {
        let jobs_dir = root.join("jobs");
        let handoffs_dir = root.join("handoffs");
        fs::create_dir_all(&jobs_dir)?;
        fs::create_dir_all(&handoffs_dir)?;

        let job_sql = SqlCatalog::jobs()?;
        let handoff_sql = SqlCatalog::handoffs()?;
        let mut jobs = Connection::open(jobs_dir.join("jobs.db"))?;
        let handoffs = Connection::open(handoffs_dir.join("handoffs.db"))?;
        jobs.busy_timeout(Duration::from_secs(10))?;
        handoffs.busy_timeout(Duration::from_secs(10))?;
        initialize_jobs(&mut jobs, &job_sql)?;
        initialize(&handoffs, &handoff_sql)?;
        let recovery_sql = SqlCatalog::parse(include_str!("../../sql/recovery_store.sql"))?;
        for statement in recovery_sql.script("init_schema")? {
            jobs.execute_batch(statement)?;
        }

        let store = Self {
            jobs: Mutex::new(jobs),
            handoffs: Mutex::new(handoffs),
            job_sql,
            handoff_sql,
            recovery_sql,
            max_retained_jobs: DEFAULT_MAX_RETAINED,
            max_retained_handoffs: DEFAULT_MAX_RETAINED,
        };
        let _ = store.reconcile_stale_running_jobs(STALE_RUNNING_SECONDS)?;
        Ok(store)
    }

    pub fn create_job(&self, request: NewJob) -> crate::Result<JobRecord> {
        let record = build_job_record(request)?;
        let mut connection = lock(&self.jobs)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        self.insert_job_on(&transaction, &record)?;
        transaction.commit()?;
        Ok(record)
    }

    pub fn resolve_or_register(
        &self,
        request: NewJob,
        retry_same_request: bool,
    ) -> crate::Result<JobRegistration> {
        validate_new_job(&request)?;
        let mut connection = lock(&self.jobs)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let explicit_id = request
            .job_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty());
        let mut existing = explicit_id
            .map(|id| load_job_by_id(&transaction, &self.job_sql, id))
            .transpose()?
            .flatten();
        if existing.is_none()
            && let Some(key) = request
                .idempotency_key
                .as_deref()
                .map(str::trim)
                .filter(|key| !key.is_empty())
        {
            existing = load_job_by_idempotency(&transaction, &self.job_sql, key)?;
        }
        if let Some(mut record) = existing {
            if record.status == JobStatus::Running {
                if !running_record_is_stale(&record, now_seconds()) {
                    transaction.commit()?;
                    return Ok(JobRegistration {
                        record,
                        should_dispatch: false,
                    });
                }
                record.status = JobStatus::TimedOutUnknown;
                record.outcome_state = OutcomeState::OutcomeUnknown;
                record.completed_at = Some(now_seconds());
                record.error =
                    Some("Orphaned running job reconciled as timed_out_unknown.".to_owned());
                self.update_job_on(&transaction, &record)?;
            }
            if !retry_same_request
                && (matches!(
                    record.status,
                    JobStatus::TimedOutUnknown | JobStatus::Completed | JobStatus::Cancelled
                ) || record.cancel_requested)
            {
                transaction.commit()?;
                return Ok(JobRegistration {
                    record,
                    should_dispatch: false,
                });
            }
            record.attempt_count = record.attempt_count.saturating_add(1);
            record.status = JobStatus::Running;
            record.outcome_state = OutcomeState::InFlight;
            record.error = None;
            record.completed_at = None;
            record.cancel_requested = false;
            record.cancelled_at = None;
            record.metadata.extend(request.metadata);
            record
                .metadata
                .insert("owner_pid".to_owned(), std::process::id().to_string());
            self.update_job_on(&transaction, &record)?;
            transaction.commit()?;
            return Ok(JobRegistration {
                record,
                should_dispatch: true,
            });
        }
        let record = build_job_record(request)?;
        self.insert_job_on(&transaction, &record)?;
        transaction.commit()?;
        Ok(JobRegistration {
            record,
            should_dispatch: true,
        })
    }

    pub fn get_job(&self, job_id: &str) -> crate::Result<Option<JobRecord>> {
        validate_text(job_id, "job ID")?;
        let connection = lock(&self.jobs)?;
        let query = self.job_sql.query("select_job_by_id", 1)?;
        let raw: Option<String> = connection
            .query_row(query, params![job_id.trim()], |row| row.get(0))
            .optional()?;
        raw.map(|json| deserialize(&json, "job record")).transpose()
    }

    pub fn find_job_by_idempotency_key(
        &self,
        idempotency_key: &str,
    ) -> crate::Result<Option<JobRecord>> {
        validate_text(idempotency_key, "idempotency key")?;
        let connection = lock(&self.jobs)?;
        let query = self.job_sql.query("select_job_by_idempotency_key", 1)?;
        let raw: Option<String> = connection
            .query_row(query, params![idempotency_key.trim()], |row| row.get(0))
            .optional()?;
        raw.map(|json| deserialize(&json, "job record")).transpose()
    }

    pub fn list_jobs(
        &self,
        limit: usize,
        workspace_dir: Option<&str>,
    ) -> crate::Result<Vec<JobRecord>> {
        let connection = lock(&self.jobs)?;
        let bounded = i64::try_from(limit.max(1)).unwrap_or(i64::MAX);
        let mut records = Vec::new();
        if let Some(path) = workspace_dir {
            let workspace = normalize_workspace(Path::new(path));
            validate_text(&workspace, "workspace path")?;
            let query = self.job_sql.query("select_jobs_by_workspace", 2)?;
            let mut statement = connection.prepare(query)?;
            let rows = statement.query_map(params![workspace, bounded], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (_, raw) = row?;
                records.push(deserialize(&raw, "job record")?);
            }
            if records.len() < limit.max(1) {
                let query = self.job_sql.query("select_jobs_excluding_workspace", 2)?;
                let mut statement = connection.prepare(query)?;
                let scan_limit = bounded.saturating_mul(4);
                let rows = statement.query_map(params![workspace, scan_limit], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let (row_workspace, raw) = row?;
                    if workspace_matches(&row_workspace, &workspace) {
                        records.push(deserialize(&raw, "job record")?);
                        if records.len() >= limit.max(1) {
                            break;
                        }
                    }
                }
            }
        } else {
            let query = self.job_sql.query("select_recent_jobs", 1)?;
            let mut statement = connection.prepare(query)?;
            let rows = statement.query_map(params![bounded], |row| row.get::<_, String>(1))?;
            for row in rows {
                records.push(deserialize(&row?, "job record")?);
            }
        }
        Ok(records)
    }

    pub fn complete_job(
        &self,
        job_id: &str,
        result: Value,
        session_id: Option<String>,
        receipt: Option<Value>,
    ) -> crate::Result<Option<JobRecord>> {
        self.complete_job_with_details(job_id, result, session_id, receipt, None, BTreeMap::new())
    }

    pub fn complete_job_with_details(
        &self,
        job_id: &str,
        result: Value,
        session_id: Option<String>,
        receipt: Option<Value>,
        model: Option<String>,
        metadata_updates: BTreeMap<String, String>,
    ) -> crate::Result<Option<JobRecord>> {
        self.transition_job(job_id, |record| {
            if record.cancel_requested
                || !matches!(
                    record.status,
                    JobStatus::Running | JobStatus::TimedOutUnknown
                )
            {
                return;
            }
            record.status = JobStatus::Completed;
            record.outcome_state = OutcomeState::ConfirmedSuccess;
            record.completed_at = Some(now_seconds());
            record.result = Some(result);
            if session_id.is_some() {
                record.session_id = session_id;
            }
            if receipt.is_some() {
                record.receipt = receipt;
            }
            if model.is_some() {
                record.model = model;
            }
            record.metadata.extend(metadata_updates);
        })
    }

    pub fn fail_job(
        &self,
        job_id: &str,
        error: String,
        receipt: Option<Value>,
    ) -> crate::Result<Option<JobRecord>> {
        self.fail_job_with_result(job_id, error, None, receipt)
    }

    pub fn fail_job_with_result(
        &self,
        job_id: &str,
        error: String,
        partial_result: Option<Value>,
        receipt: Option<Value>,
    ) -> crate::Result<Option<JobRecord>> {
        validate_text(&error, "job error")?;
        self.transition_job(job_id, |record| {
            if record.cancel_requested
                || !matches!(
                    record.status,
                    JobStatus::Running | JobStatus::TimedOutUnknown
                )
            {
                return;
            }
            record.status = JobStatus::Failed;
            record.outcome_state = OutcomeState::ConfirmedFailure;
            record.completed_at = Some(now_seconds());
            record.error = Some(error);
            if partial_result.is_some() {
                record.result = partial_result;
            }
            if receipt.is_some() {
                record.receipt = receipt;
            }
        })
    }

    pub fn mark_timed_out_unknown(
        &self,
        job_id: &str,
        error: String,
        partial_result: Option<Value>,
        receipt: Option<Value>,
    ) -> crate::Result<Option<JobRecord>> {
        validate_text(&error, "timeout error")?;
        self.transition_job(job_id, |record| {
            if record.cancel_requested || record.status != JobStatus::Running {
                return;
            }
            record.status = JobStatus::TimedOutUnknown;
            record.outcome_state = OutcomeState::OutcomeUnknown;
            record.completed_at = Some(now_seconds());
            record.error = Some(error);
            if partial_result.is_some() {
                record.result = partial_result;
            }
            if receipt.is_some() {
                record.receipt = receipt;
            }
        })
    }

    pub fn reject_before_dispatch(
        &self,
        job_id: &str,
        error: String,
        receipt: Option<Value>,
    ) -> crate::Result<Option<JobRecord>> {
        validate_text(&error, "pre-dispatch rejection error")?;
        self.transition_job(job_id, |record| {
            if record.status != JobStatus::Running || record.cancel_requested {
                return;
            }
            record.status = JobStatus::RejectedPreDispatch;
            record.outcome_state = OutcomeState::ConfirmedFailure;
            record.completed_at = Some(now_seconds());
            record.error = Some(error);
            if receipt.is_some() {
                record.receipt = receipt;
            }
        })
    }

    pub fn mark_input_budget_exceeded(
        &self,
        job_id: &str,
        error: String,
        receipt: Option<Value>,
    ) -> crate::Result<Option<JobRecord>> {
        validate_text(&error, "input budget error")?;
        self.transition_job(job_id, |record| {
            if record.status != JobStatus::Running || record.cancel_requested {
                return;
            }
            record.status = JobStatus::InputBudgetExceeded;
            record.outcome_state = OutcomeState::ConfirmedFailure;
            record.completed_at = Some(now_seconds());
            record.error = Some(error);
            if receipt.is_some() {
                record.receipt = receipt;
            }
        })
    }

    pub fn cancel_job(&self, job_id: &str, reason: String) -> crate::Result<Option<JobRecord>> {
        self.cancel_job_with_result(job_id, reason, None, None)
    }

    pub(crate) fn cancel_job_with_result(
        &self,
        job_id: &str,
        reason: String,
        partial_result: Option<Value>,
        receipt: Option<Value>,
    ) -> crate::Result<Option<JobRecord>> {
        validate_text(&reason, "cancellation reason")?;
        self.transition_job(job_id, |record| {
            if matches!(
                record.status,
                JobStatus::Running | JobStatus::TimedOutUnknown
            ) {
                let now = now_seconds();
                record.cancel_requested = true;
                record.cancelled_at = Some(now);
                record.status = JobStatus::Cancelled;
                record.outcome_state = OutcomeState::Cancelled;
                record.completed_at = Some(now);
                record.error = Some(reason);
            } else if record.status != JobStatus::Cancelled {
                return;
            }
            // A caller may have already requested cancellation before the
            // runtime returned its receipt. Enrich it without rewriting terminal evidence.
            if record.result.is_none() {
                record.result = partial_result;
            }
            if record.receipt.is_none() {
                record.receipt = receipt;
            }
        })
    }

    pub fn reconcile_stale_running_jobs(&self, stale_age_seconds: f64) -> crate::Result<usize> {
        let now = now_seconds();
        let candidates = self.jobs_by_status(JobStatus::Running)?;
        let mut count = 0;
        for record in candidates {
            if now - record.created_at < stale_age_seconds || !running_record_is_stale(&record, now)
            {
                continue;
            }
            if self
                .mark_timed_out_unknown(
                    &record.job_id,
                    record.error.unwrap_or_else(|| {
                        "Orphaned running job reconciled as timed_out_unknown.".to_owned()
                    }),
                    None,
                    record.receipt,
                )?
                .is_some()
            {
                count += 1;
            }
        }
        Ok(count)
    }

    pub fn job_telemetry(&self) -> crate::Result<JobTelemetry> {
        let connection = lock(&self.jobs)?;
        let query = self.job_sql.query("summarize_job_telemetry", 0)?;
        let counts: (i64, i64, i64, i64, i64, i64, f64) =
            connection.query_row(query, [], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            })?;
        Ok(JobTelemetry {
            total_jobs: nonnegative_count(counts.0),
            completed_jobs: nonnegative_count(counts.1),
            failed_jobs: nonnegative_count(counts.2),
            unknown_jobs: nonnegative_count(counts.3),
            running_jobs: nonnegative_count(counts.4),
            cancelled_jobs: nonnegative_count(counts.5),
            average_duration_seconds: counts.6,
        })
    }

    pub fn prune_completed_jobs(
        &self,
        max_retained: usize,
        checkpoint: bool,
    ) -> crate::Result<usize> {
        let connection = lock(&self.jobs)?;
        let query = self.job_sql.query("prune_completed_jobs", 1)?;
        let deleted = connection.execute(
            query,
            params![i64::try_from(max_retained.max(1)).unwrap_or(i64::MAX)],
        )?;
        if checkpoint {
            connection.execute_batch(self.job_sql.query("pragma_wal_checkpoint", 0)?)?;
        }
        Ok(deleted)
    }

    pub fn create_handoff(&self, request: NewHandoff) -> crate::Result<HandoffTicket> {
        validate_text(&request.title, "handoff title")?;
        validate_text(&request.spec_or_payload, "handoff payload")?;
        let now = now_seconds();
        let ticket = HandoffTicket {
            ticket_id: format!("hnd-{}", &Uuid::new_v4().simple().to_string()[..8]),
            sender: request.sender,
            recipient: request.recipient,
            title: request.title.trim().to_owned(),
            spec_or_payload: request.spec_or_payload,
            workspace_dir: normalize_workspace(&request.workspace_dir),
            priority: request.priority,
            status: HandoffStatus::Pending,
            created_at: now,
            updated_at: now,
            resolution_note: None,
            alexandria_ref: request.alexandria_ref,
        };
        self.save_handoff(&ticket)?;
        Ok(ticket)
    }

    pub fn get_handoff(&self, ticket_id: &str) -> crate::Result<Option<HandoffTicket>> {
        validate_text(ticket_id, "handoff ID")?;
        let connection = lock(&self.handoffs)?;
        let query = self.handoff_sql.query("select_handoff_by_id", 1)?;
        let raw: Option<String> = connection
            .query_row(query, params![ticket_id.trim()], |row| row.get(0))
            .optional()?;
        raw.map(|json| deserialize(&json, "handoff ticket"))
            .transpose()
    }

    pub fn update_handoff(
        &self,
        ticket_id: &str,
        status: HandoffStatus,
        resolution_note: Option<String>,
        alexandria_ref: Option<String>,
    ) -> crate::Result<Option<HandoffTicket>> {
        let Some(mut ticket) = self.get_handoff(ticket_id)? else {
            return Ok(None);
        };
        if let Some(note) = resolution_note {
            validate_text(&note, "handoff resolution")?;
            ticket.resolution_note = Some(note);
        }
        if alexandria_ref.is_some() {
            ticket.alexandria_ref = alexandria_ref;
        }
        ticket.status = status;
        ticket.updated_at = now_seconds();
        self.save_handoff(&ticket)?;
        Ok(Some(ticket))
    }

    pub fn resolve_handoff(
        &self,
        ticket_id: &str,
        status: HandoffStatus,
        summary: String,
        verdict: AuditVerdict,
        alexandria_ref: Option<String>,
    ) -> crate::Result<Option<HandoffTicket>> {
        validate_text(&summary, "handoff resolution summary")?;
        let resolution_note = serde_json::to_string(&serde_json::json!({
            "verdict": verdict,
            "summary": summary,
        }))?;
        self.update_handoff(ticket_id, status, Some(resolution_note), alexandria_ref)
    }

    pub fn list_handoffs(&self, filter: HandoffFilter) -> crate::Result<Vec<HandoffTicket>> {
        let recipient = filter.recipient.map(TriadRole::as_str);
        let status = filter.status.map(HandoffStatus::as_str);
        let bounded = i64::try_from(filter.limit.max(1)).unwrap_or(i64::MAX);
        let workspace = filter.workspace_dir.as_deref().map(normalize_workspace);
        let connection = lock(&self.handoffs)?;
        let query = self
            .handoff_sql
            .query("select_handoffs_filtered_limited", 5)?;
        let mut statement = connection.prepare(query)?;
        let rows = statement.query_map(
            params![recipient, recipient, status, status, bounded],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?;
        let mut tickets = Vec::new();
        for row in rows {
            let (row_workspace, raw) = row?;
            if workspace
                .as_ref()
                .is_some_and(|expected| !workspace_matches(&row_workspace, expected))
            {
                continue;
            }
            tickets.push(deserialize(&raw, "handoff ticket")?);
        }
        Ok(tickets)
    }

    pub fn pending_handoff_counts(&self) -> crate::Result<BTreeMap<TriadRole, u64>> {
        let connection = lock(&self.handoffs)?;
        let query = self.handoff_sql.query("count_handoffs_by_status", 1)?;
        let mut statement = connection.prepare(query)?;
        let rows = statement.query_map(params![HandoffStatus::Pending.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let mut counts = BTreeMap::new();
        for row in rows {
            let (role, count) = row?;
            if let Some(role) = parse_role(&role) {
                counts.insert(role, nonnegative_count(count));
            }
        }
        Ok(counts)
    }

    pub fn prune_resolved_handoffs(
        &self,
        max_retained: usize,
        checkpoint: bool,
    ) -> crate::Result<usize> {
        let connection = lock(&self.handoffs)?;
        let query = self.handoff_sql.query("prune_resolved_handoffs", 1)?;
        let deleted = connection.execute(
            query,
            params![i64::try_from(max_retained.max(1)).unwrap_or(i64::MAX)],
        )?;
        if checkpoint {
            connection.execute_batch(self.handoff_sql.query("pragma_wal_checkpoint", 0)?)?;
        }
        Ok(deleted)
    }

    pub fn checkpoint(&self) -> crate::Result<()> {
        let jobs = lock(&self.jobs)?;
        jobs.execute_batch(self.job_sql.query("pragma_wal_checkpoint", 0)?)?;
        let handoffs = lock(&self.handoffs)?;
        handoffs.execute_batch(self.handoff_sql.query("pragma_wal_checkpoint", 0)?)?;
        Ok(())
    }

    fn transition_job(
        &self,
        job_id: &str,
        change: impl FnOnce(&mut JobRecord),
    ) -> crate::Result<Option<JobRecord>> {
        validate_text(job_id, "job ID")?;
        let mut connection = lock(&self.jobs)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(mut record) = load_job_by_id(&transaction, &self.job_sql, job_id)? else {
            transaction.commit()?;
            return Ok(None);
        };
        change(&mut record);
        self.update_job_on(&transaction, &record)?;
        transaction.commit()?;
        Ok(Some(record))
    }

    fn jobs_by_status(&self, status: JobStatus) -> crate::Result<Vec<JobRecord>> {
        let connection = lock(&self.jobs)?;
        let query = self.job_sql.query("select_jobs_by_status", 1)?;
        let mut statement = connection.prepare(query)?;
        let rows = statement.query_map(params![status.as_str()], |row| row.get::<_, String>(0))?;
        let mut records = Vec::new();
        for row in rows {
            records.push(deserialize(&row?, "job record")?);
        }
        Ok(records)
    }

    fn insert_job_on(&self, connection: &Connection, record: &JobRecord) -> crate::Result<()> {
        validate_job(record)?;
        let serialized = serde_json::to_string(record)?;
        let query = self.job_sql.query("insert_job", 10)?;
        connection.execute(
            query,
            params![
                record.job_id,
                record.task_type,
                record.status.as_str(),
                record.outcome_state.as_str(),
                record.idempotency_key,
                record.idempotency_key,
                record.workspace_dir,
                record.created_at,
                now_seconds(),
                serialized,
            ],
        )?;
        self.prune_jobs_on(connection)
    }

    fn update_job_on(&self, connection: &Connection, record: &JobRecord) -> crate::Result<()> {
        validate_job(record)?;
        let serialized = serde_json::to_string(record)?;
        let query = self.job_sql.query("update_job", 9)?;
        let updated = connection.execute(
            query,
            params![
                record.task_type,
                record.status.as_str(),
                record.outcome_state.as_str(),
                record.idempotency_key,
                record.workspace_dir,
                record.created_at,
                now_seconds(),
                serialized,
                record.job_id,
            ],
        )?;
        if updated != 1 {
            return Err(IoError::other(format!(
                "job update affected {updated} rows for {}",
                record.job_id
            ))
            .into());
        }
        self.prune_jobs_on(connection)
    }

    fn prune_jobs_on(&self, connection: &Connection) -> crate::Result<()> {
        let prune = self.job_sql.query("prune_completed_jobs", 1)?;
        connection.execute(
            prune,
            params![i64::try_from(self.max_retained_jobs).unwrap_or(i64::MAX)],
        )?;
        Ok(())
    }

    fn save_handoff(&self, ticket: &HandoffTicket) -> crate::Result<()> {
        validate_handoff(ticket)?;
        let serialized = serde_json::to_string(ticket)?;
        let connection = lock(&self.handoffs)?;
        let query = self.handoff_sql.query("upsert_handoff", 9)?;
        connection.execute(
            query,
            params![
                ticket.ticket_id,
                ticket.sender.as_str(),
                ticket.recipient.as_str(),
                ticket.status.as_str(),
                ticket.priority.as_str(),
                ticket.workspace_dir,
                ticket.created_at,
                ticket.updated_at,
                serialized,
            ],
        )?;
        let prune = self.handoff_sql.query("prune_resolved_handoffs", 1)?;
        connection.execute(
            prune,
            params![i64::try_from(self.max_retained_handoffs).unwrap_or(i64::MAX)],
        )?;
        Ok(())
    }
}

fn initialize(connection: &Connection, catalog: &SqlCatalog) -> crate::Result<()> {
    enable_wal_with_retry(connection, catalog)?;
    for statement in catalog.script("init_schema")? {
        connection.execute_batch(statement)?;
    }
    for statement in catalog.script("pragma_connect")? {
        connection.execute_batch(statement)?;
    }
    Ok(())
}

fn enable_wal_with_retry(connection: &Connection, catalog: &SqlCatalog) -> crate::Result<()> {
    let query = catalog.query("pragma_enable_wal", 0)?;
    let mut delay = Duration::from_millis(5);
    for attempt in 0..20 {
        match connection.query_row(query, [], |row| row.get::<_, String>(0)) {
            Ok(mode) if mode.eq_ignore_ascii_case("wal") => return Ok(()),
            Ok(mode) => {
                return Err(IoError::other(format!(
                    "SQLite refused WAL journal mode and returned '{mode}'"
                ))
                .into());
            }
            Err(error) if sqlite_is_busy(&error) && attempt < 19 => {
                std::thread::sleep(delay);
                delay = delay.saturating_mul(2).min(Duration::from_millis(100));
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(IoError::other("SQLite WAL initialization retry budget exhausted").into())
}

fn sqlite_is_busy(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(details, _)
            if matches!(details.code, ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked)
    )
}

fn initialize_jobs(connection: &mut Connection, catalog: &SqlCatalog) -> crate::Result<()> {
    initialize(connection, catalog)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let columns_query = catalog.query("pragma_job_columns", 0)?;
    let mut columns = transaction.prepare(columns_query)?;
    let column_names = columns
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(columns);
    if !column_names.iter().any(|name| name == "idempotency_owner") {
        transaction.execute(catalog.query("alter_add_idempotency_owner", 0)?, [])?;
    }
    let indexes_query = catalog.query("pragma_job_indexes", 0)?;
    let mut indexes = transaction.prepare(indexes_query)?;
    let index_names = indexes
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(indexes);
    if !index_names
        .iter()
        .any(|name| name == "uq_jobs_idempotency_owner")
    {
        transaction.execute(catalog.query("assign_idempotency_owners", 0)?, [])?;
        transaction.execute(
            catalog.query("create_unique_idempotency_owner_index", 0)?,
            [],
        )?;
    }
    transaction.commit()?;
    Ok(())
}

fn build_job_record(request: NewJob) -> crate::Result<JobRecord> {
    validate_new_job(&request)?;
    let mut metadata = request.metadata;
    metadata.insert("owner_pid".to_owned(), std::process::id().to_string());
    Ok(JobRecord {
        job_id: normalize_id(request.job_id, "cdx", 10),
        task_type: request.task_type,
        status: JobStatus::Running,
        created_at: request.created_at.unwrap_or_else(now_seconds),
        workspace_dir: normalize_workspace(&request.workspace_dir),
        model: request.model,
        session_id: None,
        completed_at: None,
        result: None,
        error: None,
        metadata,
        outcome_state: OutcomeState::InFlight,
        idempotency_key: normalize_optional(request.idempotency_key),
        provider: normalize_optional(request.provider),
        engine: normalize_optional(request.engine),
        attempt_count: 1,
        cancel_requested: false,
        cancelled_at: None,
        receipt: None,
    })
}

fn load_job_by_id(
    connection: &Connection,
    catalog: &SqlCatalog,
    job_id: &str,
) -> crate::Result<Option<JobRecord>> {
    validate_text(job_id, "job ID")?;
    let query = catalog.query("select_job_by_id", 1)?;
    let raw: Option<String> = connection
        .query_row(query, params![job_id.trim()], |row| row.get(0))
        .optional()?;
    raw.map(|json| deserialize(&json, "job record")).transpose()
}

fn load_job_by_idempotency(
    connection: &Connection,
    catalog: &SqlCatalog,
    idempotency_key: &str,
) -> crate::Result<Option<JobRecord>> {
    validate_text(idempotency_key, "idempotency key")?;
    let query = catalog.query("select_job_by_idempotency_key", 1)?;
    let raw: Option<String> = connection
        .query_row(query, params![idempotency_key.trim()], |row| row.get(0))
        .optional()?;
    raw.map(|json| deserialize(&json, "job record")).transpose()
}

fn lock<T>(mutex: &Mutex<T>) -> crate::Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| IoError::other("SQLite store lock was poisoned").into())
}

fn validate_new_job(request: &NewJob) -> crate::Result<()> {
    validate_text(&request.task_type, "task type")?;
    validate_text(&request.workspace_dir.to_string_lossy(), "workspace path")?;
    for value in request
        .model
        .iter()
        .chain(request.job_id.iter())
        .chain(request.idempotency_key.iter())
        .chain(request.provider.iter())
        .chain(request.engine.iter())
    {
        validate_text(value, "job field")?;
    }
    for (key, value) in &request.metadata {
        validate_text(key, "metadata key")?;
        validate_text(value, "metadata value")?;
    }
    Ok(())
}

fn validate_job(record: &JobRecord) -> crate::Result<()> {
    for value in [&record.job_id, &record.task_type, &record.workspace_dir] {
        validate_text(value, "job record")?;
    }
    Ok(())
}

fn validate_handoff(ticket: &HandoffTicket) -> crate::Result<()> {
    for value in [
        &ticket.ticket_id,
        &ticket.title,
        &ticket.spec_or_payload,
        &ticket.workspace_dir,
    ] {
        validate_text(value, "handoff ticket")?;
    }
    Ok(())
}

fn deserialize<T: serde::de::DeserializeOwned>(raw: &str, kind: &str) -> crate::Result<T> {
    serde_json::from_str(raw).map_err(|error| {
        IoError::new(
            ErrorKind::InvalidData,
            format!("invalid persisted {kind}: {error}"),
        )
        .into()
    })
}

fn normalize_id(explicit: Option<String>, prefix: &str, length: usize) -> String {
    explicit
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| {
            format!(
                "{prefix}-{}",
                &Uuid::new_v4().simple().to_string()[..length]
            )
        })
}

fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
}

fn normalize_workspace(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path))
        .to_string_lossy()
        .into_owned()
}

fn workspace_matches(record_workspace: &str, expected: &str) -> bool {
    if record_workspace == expected {
        return true;
    }
    let record_path = Path::new(record_workspace);
    !record_workspace.trim().is_empty()
        && !record_path.is_absolute()
        && normalize_workspace(record_path) == expected
}

fn running_record_is_stale(record: &JobRecord, now: f64) -> bool {
    let current_pid = std::process::id().to_string();
    match record.metadata.get("owner_pid") {
        Some(pid) if pid == &current_pid => false,
        _ => now - record.created_at >= STALE_RUNNING_SECONDS,
    }
}

fn parse_role(value: &str) -> Option<TriadRole> {
    match value {
        "argon" => Some(TriadRole::Argon),
        "sonnet" => Some(TriadRole::Sonnet),
        "opus" => Some(TriadRole::Opus),
        "codex" => Some(TriadRole::Codex),
        _ => None,
    }
}

fn nonnegative_count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

fn now_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |duration| duration.as_secs_f64())
}

#[cfg(test)]
mod tests;
