// SPDX-License-Identifier: BUSL-1.1

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) struct TempDirGuard {
    pub(crate) path: PathBuf,
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.path).ok();
    }
}

pub(crate) fn unique_temp_dir(prefix: &str, label: &str) -> Result<TempDirGuard> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("Failed to compute unique timestamp")?
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "{}-{}-{}-{}",
        prefix,
        label,
        std::process::id(),
        unique
    ));
    std::fs::create_dir_all(&path)
        .with_context(|| format!("Failed to create temp dir '{}'", path.display()))?;
    Ok(TempDirGuard { path })
}
