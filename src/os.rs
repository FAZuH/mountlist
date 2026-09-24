use std::ffi::OsString;
use std::fs::OpenOptions;
use std::fs::{self};
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use serde::Deserialize;
use thiserror::Error;

use crate::DesiredMount;
use crate::Manifest;
use crate::MountIdentity;
use crate::ObservedMount;
use crate::TargetKind;
use crate::TargetState;

#[derive(Debug)]
pub struct SystemSnapshot {
    desired: Vec<DesiredMount>,
    observed: Vec<ObservedMount>,
    targets: Vec<TargetState>,
}

impl SystemSnapshot {
    pub fn desired(&self) -> &[DesiredMount] {
        &self.desired
    }

    pub fn observed(&self) -> &[ObservedMount] {
        &self.observed
    }

    pub fn targets(&self) -> &[TargetState] {
        &self.targets
    }
}

pub fn inspect_system(manifest: &Manifest) -> Result<SystemSnapshot, SystemError> {
    let observed = observed_mounts(manifest.root())?;
    let mut desired = Vec::with_capacity(manifest.shares().len());
    let mut targets = Vec::with_capacity(manifest.shares().len());
    for share in manifest.shares() {
        let source =
            fs::canonicalize(share.source()).map_err(|error| SystemError::InspectSource {
                path: share.source().to_owned(),
                error,
            })?;
        let source_identity = source_identity(&source)?;
        desired.push(DesiredMount::new(
            share.key(),
            share.source(),
            share.target(),
            source_identity,
        ));
        targets.push(inspect_target(share.target())?);
    }
    Ok(SystemSnapshot {
        desired,
        observed,
        targets,
    })
}

pub fn prepare_target(source: &Path, target: &Path) -> Result<(), MountError> {
    let metadata = fs::metadata(source).map_err(|error| MountError::InspectSource {
        path: source.to_owned(),
        error,
    })?;
    let parent = target.parent().ok_or_else(|| MountError::InvalidTarget {
        path: target.to_owned(),
    })?;
    fs::create_dir_all(parent).map_err(|error| MountError::CreateTarget {
        path: target.to_owned(),
        error,
    })?;
    if metadata.is_dir() {
        match fs::create_dir(target) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if inspect_target(target)?.kind() == TargetKind::EmptyDirectory {
                    Ok(())
                } else {
                    Err(MountError::CreateTarget {
                        path: target.to_owned(),
                        error,
                    })
                }
            }
            Err(error) => Err(MountError::CreateTarget {
                path: target.to_owned(),
                error,
            }),
        }
    } else if metadata.is_file() {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)
            .map(|_| ())
            .map_err(|error| MountError::CreateTarget {
                path: target.to_owned(),
                error,
            })
    } else {
        Err(MountError::UnsupportedSource {
            path: source.to_owned(),
        })
    }
}

pub fn mount_bind(source: &Path, target: &Path) -> Result<(), MountError> {
    run_mount_command("mount", &["--bind", "--"], source, target)
}

pub fn unmount(target: &Path) -> Result<(), MountError> {
    let output = Command::new("umount")
        .arg("--")
        .arg(target)
        .output()
        .map_err(MountError::Spawn)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(MountError::Command {
            command: "umount",
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

pub fn require_root() -> Result<(), PrivilegeError> {
    let status = fs::read_to_string("/proc/self/status").map_err(PrivilegeError::Read)?;
    let effective_uid = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|ids| ids.split_whitespace().nth(1))
        .ok_or(PrivilegeError::MissingUid)?;
    if effective_uid == "0" {
        Ok(())
    } else {
        Err(PrivilegeError::RootRequired)
    }
}

fn run_mount_command(
    command: &'static str,
    options: &[&str],
    source: &Path,
    target: &Path,
) -> Result<(), MountError> {
    let output = Command::new(command)
        .args(options)
        .arg(source)
        .arg(target)
        .output()
        .map_err(MountError::Spawn)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(MountError::Command {
            command,
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

pub fn inspect_target(path: &Path) -> Result<TargetState, StateError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(TargetState::new(path, TargetKind::Missing));
        }
        Err(error) => {
            return Err(StateError::Inspect {
                path: path.into(),
                error,
            });
        }
    };
    let kind = if metadata.file_type().is_symlink() {
        TargetKind::Symlink
    } else if metadata.is_dir() {
        let mut entries = fs::read_dir(path).map_err(|error| StateError::Inspect {
            path: path.into(),
            error,
        })?;
        match entries
            .next()
            .transpose()
            .map_err(|error| StateError::Inspect {
                path: path.into(),
                error,
            })? {
            Some(_) => TargetKind::NonEmptyDirectory,
            None => TargetKind::EmptyDirectory,
        }
    } else if metadata.is_file() {
        TargetKind::File
    } else {
        TargetKind::Other
    };
    Ok(TargetState::new(path, kind))
}

fn observed_mounts(root: &Path) -> Result<Vec<ObservedMount>, FindmntError> {
    let mounts = run_findmnt([
        OsString::from("--json"),
        OsString::from("--list"),
        OsString::from("--output"),
        OsString::from("TARGET,MAJ:MIN,FSROOT"),
    ])?;
    Ok(mounts
        .filesystems
        .into_iter()
        .filter(|mount| mount.target != root && mount.target.starts_with(root))
        .map(|mount| {
            ObservedMount::new(
                mount.target,
                MountIdentity::new(&mount.device, mount.filesystem_root),
            )
        })
        .collect())
}

fn source_identity(source: &Path) -> Result<MountIdentity, FindmntError> {
    let mount = run_findmnt([
        OsString::from("--json"),
        OsString::from("--evaluate"),
        OsString::from("--target"),
        source.as_os_str().to_owned(),
        OsString::from("--output"),
        OsString::from("TARGET,MAJ:MIN,FSROOT"),
    ])?
    .filesystems
    .into_iter()
    .next()
    .ok_or(FindmntError::MissingEvaluation(source.to_owned()))?;
    let relative = source
        .strip_prefix(&mount.target)
        .map_err(|_| FindmntError::InvalidEvaluation(source.to_owned()))?;
    let filesystem_root = mount.filesystem_root.join(relative);
    Ok(MountIdentity::new(&mount.device, filesystem_root))
}

fn run_findmnt<const N: usize>(args: [OsString; N]) -> Result<FindmntOutput, FindmntError> {
    let output = Command::new("findmnt")
        .args(args)
        .output()
        .map_err(FindmntError::Spawn)?;
    if !output.status.success() {
        return Err(FindmntError::Failed {
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    serde_yml::from_slice(&output.stdout).map_err(FindmntError::Parse)
}

#[derive(Debug, Deserialize)]
struct FindmntOutput {
    filesystems: Vec<FindmntMount>,
}

#[derive(Debug, Deserialize)]
struct FindmntMount {
    target: PathBuf,
    #[serde(rename = "maj:min")]
    device: String,
    #[serde(rename = "fsroot")]
    filesystem_root: PathBuf,
}

#[derive(Debug, Error)]
pub enum FindmntError {
    #[error("failed to start findmnt: {0}")]
    Spawn(std::io::Error),
    #[error("findmnt failed with {status}: {stderr}")]
    Failed {
        status: std::process::ExitStatus,
        stderr: String,
    },
    #[error("failed to parse findmnt output: {0}")]
    Parse(serde_yml::Error),
    #[error("findmnt returned no containing mount for {}", .0.display())]
    MissingEvaluation(PathBuf),
    #[error("findmnt returned a containing mount outside source path {}", .0.display())]
    InvalidEvaluation(PathBuf),
}

#[derive(Debug, Error)]
pub enum StateError {
    #[error("failed to inspect target {}: {error}", .path.display())]
    Inspect {
        path: PathBuf,
        #[source]
        error: std::io::Error,
    },
}

#[derive(Debug, Error)]
pub enum SystemError {
    #[error(transparent)]
    State(#[from] StateError),
    #[error(transparent)]
    Findmnt(#[from] FindmntError),
    #[error("failed to inspect source {}: {error}", .path.display())]
    InspectSource {
        path: PathBuf,
        #[source]
        error: std::io::Error,
    },
}

#[derive(Debug, Error)]
pub enum MountError {
    #[error(transparent)]
    State(#[from] StateError),
    #[error("failed to inspect source {}: {error}", .path.display())]
    InspectSource {
        path: PathBuf,
        #[source]
        error: std::io::Error,
    },
    #[error("target has no parent directory: {}", .path.display())]
    InvalidTarget { path: PathBuf },
    #[error("failed to create target {}: {error}", .path.display())]
    CreateTarget {
        path: PathBuf,
        #[source]
        error: std::io::Error,
    },
    #[error("unsupported source type: {}", .path.display())]
    UnsupportedSource { path: PathBuf },
    #[error("failed to start mount command: {0}")]
    Spawn(std::io::Error),
    #[error("{command} failed with {status}: {stderr}")]
    Command {
        command: &'static str,
        status: std::process::ExitStatus,
        stderr: String,
    },
}

#[derive(Debug, Error)]
pub enum PrivilegeError {
    #[error("failed to read process credentials: {0}")]
    Read(std::io::Error),
    #[error("process credentials do not contain an effective UID")]
    MissingUid,
    #[error("this command must run as root; mountlist does not invoke sudo")]
    RootRequired,
}
