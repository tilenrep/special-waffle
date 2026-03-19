use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Persisted checkpoint so interrupted runs can resume.
#[derive(Serialize, Deserialize, Debug, Default)]
struct Checkpoint {
    completed: HashSet<String>,
    failed: HashSet<String>,
}

pub struct Progress {
    checkpoint: Checkpoint,
    path: PathBuf,
}

impl Progress {
    /// Load an existing checkpoint file, or start fresh if it doesn't exist.
    pub fn load(path: &Path) -> Self {
        let checkpoint = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { checkpoint, path: path.to_owned() }
    }

    pub fn is_done(&self, key: &str) -> bool {
        self.checkpoint.completed.contains(key)
    }

    pub fn mark_completed(&mut self, key: &str) -> Result<()> {
        self.checkpoint.failed.remove(key);
        self.checkpoint.completed.insert(key.to_owned());
        self.save()
    }

    pub fn mark_failed(&mut self, key: &str) -> Result<()> {
        self.checkpoint.failed.insert(key.to_owned());
        self.save()
    }

    pub fn completed_count(&self) -> usize {
        self.checkpoint.completed.len()
    }

    pub fn failed_keys(&self) -> Vec<&str> {
        self.checkpoint.failed.iter().map(String::as_str).collect()
    }

    /// Reset the checkpoint (clear all state).
    pub fn reset(&mut self) -> Result<()> {
        self.checkpoint = Checkpoint::default();
        self.save()
    }

    /// Atomic write: write to a temp file then rename.
    fn save(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.checkpoint)
            .context("Failed to serialize checkpoint")?;
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, &json)
            .with_context(|| format!("Failed to write checkpoint tmp file {}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .with_context(|| format!("Failed to rename checkpoint to {}", self.path.display()))
    }
}
