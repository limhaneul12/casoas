use std::io::{Error as IoError, ErrorKind};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlCommand {
    Status,
    Setup,
    Sync,
}

impl ControlCommand {
    pub fn parse(value: &str) -> crate::Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "status" | "st" => Ok(Self::Status),
            "setup" | "install" => Ok(Self::Setup),
            "sync" | "refresh" => Ok(Self::Sync),
            other => Err(IoError::new(
                ErrorKind::InvalidInput,
                format!("unsupported lifecycle command '{other}'"),
            )
            .into()),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct InstallChanges {
    pub legacy_mcp: super::config::LegacyMcpCleanup,
    pub assets_synchronized: bool,
    pub launcher_migrated: bool,
}
