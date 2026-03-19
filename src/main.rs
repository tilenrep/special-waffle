mod api;
mod config;
mod generator;
mod progress;
mod state;

use anyhow::{bail, Context, Result};
use clap::Parser;
use std::path::PathBuf;

/// Terraform Settings Mapper
///
/// Reads Terraform remote state for one or more workspaces and generates
/// full Terraform resource blocks (.tf) from the discovered configuration.
///
/// Single-workspace (no config file):
///   terraform-settings-mapper --token <tok> --org myorg --workspace prod
///
/// Multi-workspace (config file):
///   terraform-settings-mapper --config mapper.toml
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Config file listing multiple workspaces (TOML). Mutually exclusive
    /// with --org / --workspace.
    #[arg(short, long, conflicts_with_all = ["org", "workspace"])]
    config: Option<PathBuf>,

    // ── single-workspace flags ──────────────────────────────────────────────
    /// Terraform Cloud API token (read access). Falls back to $TF_TOKEN.
    #[arg(short, long, env = "TF_TOKEN")]
    token: Option<String>,

    /// Organization name (single-workspace mode).
    #[arg(short, long, env = "TF_ORG")]
    org: Option<String>,

    /// Workspace name (single-workspace mode).
    #[arg(short, long, env = "TF_WORKSPACE")]
    workspace: Option<String>,

    /// Terraform Cloud API base URL.
    #[arg(long, default_value = "https://app.terraform.io")]
    api_url: String,

    // ── output / resume ─────────────────────────────────────────────────────
    /// Directory where generated .tf files are written (default: current dir).
    #[arg(long, default_value = ".")]
    output_dir: PathBuf,

    /// Checkpoint file used to track completed workspaces for resume support.
    #[arg(long, default_value = ".mapper-progress.json")]
    checkpoint: PathBuf,

    /// Clear the checkpoint and restart from scratch.
    #[arg(long)]
    reset: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // ── build workspace list ────────────────────────────────────────────────
    let (targets, cfg_defaults) = if let Some(config_path) = &args.config {
        let cfg = config::load(config_path)?;
        let defaults = cfg.defaults;
        (cfg.workspaces, Some(defaults))
    } else {
        let org = args.org.as_deref().context("--org is required in single-workspace mode")?;
        let workspace = args.workspace.as_deref().context("--workspace is required in single-workspace mode")?;
        let token = args.token.clone();
        let target = config::WorkspaceTarget {
            org: org.to_owned(),
            workspace: workspace.to_owned(),
            token,
            api_url: Some(args.api_url.clone()),
            output_file: None,
        };
        (vec![target], None)
    };

    let total = targets.len();
    if total == 0 {
        bail!("No workspaces configured.");
    }

    // ── checkpoint ──────────────────────────────────────────────────────────
    std::fs::create_dir_all(&args.output_dir)
        .with_context(|| format!("Cannot create output dir {}", args.output_dir.display()))?;

    let mut progress = progress::Progress::load(&args.checkpoint);
    if args.reset {
        progress.reset()?;
        eprintln!("Checkpoint reset.");
    }

    // ── process each workspace ──────────────────────────────────────────────
    let cfg_defaults = cfg_defaults.as_ref();

    for (idx, target) in targets.iter().enumerate() {
        let key = target.key();
        let label = format!("[{}/{}] {}", idx + 1, total, key);

        if progress.is_done(&key) {
            eprintln!("{label}  (already done, skipping)");
            continue;
        }

        eprintln!("{label}  Fetching state...");

        // resolve per-workspace credentials
        let token = match target.effective_token(
            args.token.as_deref(),
            cfg_defaults.and_then(|d| d.token.as_deref()),
        ) {
            Some(t) => t,
            None => {
                eprintln!("{label}  ERROR: no token available for this workspace. Skipping.");
                progress.mark_failed(&key)?;
                continue;
            }
        };

        let api_url = target.effective_api_url(
            cfg_defaults.and_then(|d| d.api_url.as_deref()),
        );

        let client = match api::TfcClient::new(&token, &api_url) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{label}  ERROR building client: {e:#}");
                progress.mark_failed(&key)?;
                continue;
            }
        };

        let state_json = match client.fetch_state(&target.org, &target.workspace).await {
            Ok(j) => j,
            Err(e) => {
                eprintln!("{label}  ERROR fetching state: {e:#}");
                progress.mark_failed(&key)?;
                continue;
            }
        };

        eprintln!("{label}  Parsing resources...");
        let resources = match state::parse_state(&state_json) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{label}  ERROR parsing state: {e:#}");
                progress.mark_failed(&key)?;
                continue;
            }
        };
        eprintln!("{label}  Found {} resource(s). Generating HCL...", resources.len());

        let hcl = generator::generate_hcl(&resources);

        // output directory: workspace override → config default → CLI flag
        let out_dir = cfg_defaults
            .and_then(|d| d.output_dir.as_deref())
            .map(PathBuf::from)
            .unwrap_or_else(|| args.output_dir.clone());

        let out_path = out_dir.join(target.output_filename());

        if let Err(e) = std::fs::write(&out_path, &hcl) {
            eprintln!("{label}  ERROR writing {}: {e:#}", out_path.display());
            progress.mark_failed(&key)?;
            continue;
        }

        eprintln!("{label}  Written → {}", out_path.display());
        progress.mark_completed(&key)?;
    }

    // ── summary ─────────────────────────────────────────────────────────────
    let failed = progress.failed_keys();
    eprintln!();
    eprintln!(
        "Done. {}/{} workspace(s) completed successfully.",
        progress.completed_count(),
        total
    );
    if !failed.is_empty() {
        eprintln!("Failed workspaces (will be retried on next run):");
        for f in &failed {
            eprintln!("  - {f}");
        }
    }

    Ok(())
}
