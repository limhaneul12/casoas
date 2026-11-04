//! Tri-polar orchestration, evidence adjudication, and decision packaging.

mod adjudication;
mod decision;
mod gates;
mod orchestrator;
mod types;

pub use decision::decision_pack;
pub use orchestrator::orchestrate;
