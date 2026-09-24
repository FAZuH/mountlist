use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;
use mountlist::Manifest;
use mountlist::ManifestError;
use mountlist::MountAction;
use mountlist::MountError;
use mountlist::Plan;
use mountlist::PrivilegeError;
use mountlist::ReconcileError;
use mountlist::SystemError;
use mountlist::inspect_system;
use mountlist::mount_bind;
use mountlist::prepare_target;
use mountlist::reconcile;
use mountlist::require_root;
use mountlist::unmount;
use thiserror::Error;

#[derive(Debug, Parser)]
#[command(
    name = "mountlist",
    version,
    about = "Reconcile bind mounts from a YAML manifest"
)]
struct Cli {
    #[arg(long, global = true, value_name = "PATH")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Plan,
    Status,
    Apply,
    Prune {
        #[arg(long, required = true, action = clap::ArgAction::SetTrue)]
        _yes: bool,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), CliError> {
    if matches!(cli.command, Command::Apply | Command::Prune { .. }) {
        require_root()?;
    }
    let config = cli.config.map_or_else(default_config_path, Ok)?;
    let manifest = Manifest::load(&config)?;
    let snapshot = inspect_system(&manifest)?;
    let plan = reconcile(snapshot.desired(), snapshot.observed(), snapshot.targets())?;

    match cli.command {
        Command::Plan => {
            print_plan(&plan, OutputMode::Plan);
            Ok(())
        }
        Command::Status => {
            print_plan(&plan, OutputMode::Status);
            Ok(())
        }
        Command::Apply => apply(&plan),
        Command::Prune { .. } => prune(&plan),
    }
}

fn default_config_path() -> Result<PathBuf, CliError> {
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .ok_or(CliError::HomeUnavailable)?;
    Ok(PathBuf::from(home).join(".config/shared-mounts/config.yaml"))
}

fn apply(plan: &Plan) -> Result<(), CliError> {
    for entry in plan.entries() {
        if entry.action() != MountAction::Mount {
            continue;
        }
        prepare_target(entry.source(), entry.target())?;
        mount_bind(entry.source(), entry.target())?;
        println!("mounted {} at {}", entry.key(), entry.target().display());
    }
    Ok(())
}

fn prune(plan: &Plan) -> Result<(), CliError> {
    for target in plan.prune_targets() {
        unmount(target)?;
        println!("unmounted stale mount at {}", target.display());
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum OutputMode {
    Plan,
    Status,
}

fn print_plan(plan: &Plan, mode: OutputMode) {
    for entry in plan.entries() {
        let label = match mode {
            OutputMode::Plan if entry.action() == MountAction::Mount => "mount",
            OutputMode::Plan => "mounted",
            OutputMode::Status if entry.state() == mountlist::MountState::Mounted => "mounted",
            OutputMode::Status => "missing",
        };
        println!(
            "{label} {} {} -> {}",
            entry.key(),
            entry.source().display(),
            entry.target().display()
        );
    }
    for target in plan.prune_targets() {
        println!("stale mount {}", target.display());
    }
}

#[derive(Debug, Error)]
enum CliError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    System(#[from] SystemError),
    #[error(transparent)]
    Reconcile(#[from] ReconcileError),
    #[error(transparent)]
    Mount(#[from] MountError),
    #[error(transparent)]
    Privilege(#[from] PrivilegeError),
    #[error("HOME is not set")]
    HomeUnavailable,
}
