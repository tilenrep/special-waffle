use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

/// Top-level config file (`mapper.toml`).
///
/// Example:
/// ```toml
/// [defaults]
/// token     = "..."
/// api_url   = "https://app.terraform.io"
/// output_dir = "output"
///
/// [[workspaces]]
/// org       = "my-org"
/// workspace = "production"
///
/// [[workspaces]]
/// org       = "other-org"
/// workspace = "shared-infra"
/// token     = "different-token"          # per-workspace override
/// api_url   = "https://tfe.mycompany.com" # different TFE instance
/// ```
#[derive(Deserialize, Debug)]
pub struct Config {
    #[serde(default)]
    pub defaults: Defaults,
    pub workspaces: Vec<WorkspaceTarget>,
}

#[derive(Deserialize, Debug, Default)]
pub struct Defaults {
    pub token: Option<String>,
    pub api_url: Option<String>,
    pub output_dir: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct WorkspaceTarget {
    pub org: String,
    pub workspace: String,
    /// Per-workspace token override.
    pub token: Option<String>,
    /// Per-workspace API base URL (e.g. a private TFE instance).
    pub api_url: Option<String>,
    /// Custom output filename. Defaults to `<org>__<workspace>.tf`.
    pub output_file: Option<String>,
}

impl WorkspaceTarget {
    /// Stable identifier used in checkpoint files.
    pub fn key(&self) -> String {
        format!("{}/{}", self.org, self.workspace)
    }

    /// Resolve token: workspace override → config default → env var `TF_TOKEN` → CLI flag.
    pub fn effective_token<'a>(&'a self, cli_token: Option<&'a str>, config_default: Option<&'a str>) -> Option<String> {
        self.token
            .as_deref()
            .or(config_default)
            .or(cli_token)
            .map(str::to_owned)
    }

    /// Resolve API URL: workspace override → config default → hard-coded TFC URL.
    pub fn effective_api_url(&self, config_default: Option<&str>) -> String {
        self.api_url
            .as_deref()
            .or(config_default)
            .unwrap_or("https://app.terraform.io")
            .trim_end_matches('/')
            .to_owned()
    }

    /// Output file name within the output directory.
    pub fn output_filename(&self) -> String {
        self.output_file
            .clone()
            .unwrap_or_else(|| format!("{}_{}.tf", self.org, self.workspace))
    }
}

pub fn load(path: &Path) -> Result<Config> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Cannot read config file {}", path.display()))?;
    toml::from_str(&raw)
        .with_context(|| format!("Failed to parse config file {}", path.display()))
}
