use std::io::{Error as IoError, ErrorKind};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlCommand {
    Status,
    Setup,
    Enable,
    Disable,
    Restart,
    Sync,
}

impl ControlCommand {
    pub fn parse(value: &str) -> crate::Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "status" | "st" => Ok(Self::Status),
            "setup" | "install" => Ok(Self::Setup),
            "on" | "enable" | "start" => Ok(Self::Enable),
            "off" | "disable" | "stop" => Ok(Self::Disable),
            "restart" | "reload" => Ok(Self::Restart),
            "sync" | "refresh" => Ok(Self::Sync),
            other => Err(IoError::new(
                ErrorKind::InvalidInput,
                format!("unsupported lifecycle command '{other}'"),
            )
            .into()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OwnedProcessHandle {
    pub pid: u32,
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    #[serde(default)]
    pub start_identity: String,
    pub registered_at_unix_ms: u128,
    /// Startup disk observation, not a proof of the process's loaded image.
    #[serde(default)]
    pub startup_binary_sha256: Option<String>,
    /// Version of the registering package; absent for external PID registration.
    #[serde(default)]
    pub registrant_package_version: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct InstallChanges {
    pub configuration_changed: bool,
    pub assets_synchronized: bool,
    pub launcher_migrated: bool,
}
