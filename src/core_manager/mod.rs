mod inventory;
mod storage;

#[cfg(test)]
mod tests;

use crate::core::CoreVersion;
use std::path::{Path, PathBuf};

pub(crate) use inventory::{binary_version, managed_active_version};
pub(crate) use storage::{install_version, switch_current};

pub const MANAGED_ROOT: &str = "/usr/lib/mihomo-tui";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorePaths {
    root: PathBuf,
    cores: PathBuf,
    current: PathBuf,
}

impl CorePaths {
    pub fn system() -> Self {
        Self::under(Path::new(MANAGED_ROOT))
    }

    pub(crate) fn under(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            cores: root.join("cores"),
            current: root.join("current"),
        }
    }

    pub fn binary(&self, version: CoreVersion) -> PathBuf {
        self.cores.join(version.to_string()).join("mihomo")
    }

    pub(crate) fn active_binary(&self) -> PathBuf {
        self.current.join("mihomo")
    }
}

pub fn status() -> Result<String, String> {
    inventory::status()
}
