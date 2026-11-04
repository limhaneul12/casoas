//! Durable SQLite storage and SQL boundary defenses.

mod domain;
mod sql;
mod store;

pub use domain::{
    AuditVerdict, HandoffFilter, HandoffPriority, HandoffStatus, HandoffTicket, JobRecord,
    JobRegistration, JobStatus, JobTelemetry, NewHandoff, NewJob, OutcomeState, TriadRole,
};
pub use sql::{
    CodegraphNode, CodegraphReader, SqlCatalog, escape_like_suffix, open_readonly_query_only,
    readonly_sqlite_uri,
};
pub use store::{RecoveryRecord, Store};
