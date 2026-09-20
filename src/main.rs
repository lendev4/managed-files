use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod apply;
mod formats;
mod manifest;

#[derive(Debug, Parser)]
#[command(version, about = "Manage mutable configuration files declaratively")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Validate all files, then apply the manifest in order.
    Apply {
        manifest: PathBuf,
        /// Preview actions and backups without changing files or directories.
        #[arg(long)]
        dry_run: bool,
    },
}

fn main() -> Result<()> {
    let Cli {
        command: Command::Apply { manifest, dry_run },
    } = Cli::parse();
    let contents = std::fs::read_to_string(&manifest)
        .with_context(|| format!("failed to read manifest {}", manifest.display()))?;
    let parsed: manifest::Manifest = serde_json::from_str(&contents)
        .with_context(|| format!("failed to parse manifest {}", manifest.display()))?;
    let plan = apply::Plan::prepare(&parsed, &manifest)?;
    plan.check()?;
    for file in &plan.files {
        if !dry_run {
            file.apply()
                .with_context(|| format!("failed to manage {}", file.target().display()))?;
        }
        let backup = file
            .backup()
            .map(|path| format!(" (backup: {})", path.display()))
            .unwrap_or_default();
        println!(
            "{} {}{}",
            file.action.label(dry_run),
            file.target().display(),
            backup
        );
    }
    Ok(())
}
