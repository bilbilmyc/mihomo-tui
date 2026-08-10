mod manifest;
mod version;

#[cfg(test)]
mod tests;

use serde::Deserialize;

pub use manifest::{CorePackage, CoreRelease};
pub use version::CoreVersion;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compatibility {
    Supported,
    TooOld,
    UntestedNewer,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CoreLicense {
    pub spdx: String,
    pub asset: String,
    pub sha256: String,
}
