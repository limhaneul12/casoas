//! Tri-polar orchestration, evidence adjudication, and decision packaging.

mod adjudication;
mod decision;
mod gates;
mod orchestrator;
mod types;

#[cfg(test)]
pub(crate) use adjudication::{adjudicate, canonical_digest, inspect_role};
pub use decision::decision_pack;
pub(crate) use decision::{load_source, recompute};
pub use orchestrator::orchestrate;
