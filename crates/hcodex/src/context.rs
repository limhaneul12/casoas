//! Skill discovery and workspace-grounded prompt context.

mod relevance;
mod skills;
mod source;

pub use relevance::{RelevanceCandidate, RelevanceContext, discover_relevance};
pub use skills::{SkillEntry, SkillRegistry, SkillScope};
pub use source::{GroundedContext, SourceContextBuilder, SourceExcerpt};
