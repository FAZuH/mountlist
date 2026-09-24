mod config;
mod os;

use std::path::Path;
use std::path::PathBuf;

pub use config::Manifest;
pub use config::ManifestError;
pub use config::Share;
pub use os::FindmntError;
pub use os::MountError;
pub use os::PrivilegeError;
pub use os::StateError;
pub use os::SystemError;
pub use os::inspect_system;
pub use os::inspect_target;
pub use os::mount_bind;
pub use os::prepare_target;
pub use os::require_root;
pub use os::unmount;
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MountIdentity {
    device: String,
    filesystem_root: PathBuf,
}

impl MountIdentity {
    pub fn new(device: &str, filesystem_root: impl Into<PathBuf>) -> Self {
        Self {
            device: device.to_owned(),
            filesystem_root: filesystem_root.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesiredMount {
    key: String,
    source: PathBuf,
    target: PathBuf,
    source_identity: MountIdentity,
}

impl DesiredMount {
    pub fn new(
        key: &str,
        source: impl Into<PathBuf>,
        target: impl Into<PathBuf>,
        source_identity: MountIdentity,
    ) -> Self {
        Self {
            key: key.to_owned(),
            source: source.into(),
            target: target.into(),
            source_identity,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservedMount {
    target: PathBuf,
    source_identity: MountIdentity,
}

impl ObservedMount {
    pub fn new(target: impl Into<PathBuf>, source_identity: MountIdentity) -> Self {
        Self {
            target: target.into(),
            source_identity,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetKind {
    Missing,
    EmptyDirectory,
    NonEmptyDirectory,
    File,
    Symlink,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetState {
    path: PathBuf,
    kind: TargetKind,
}

impl TargetState {
    pub fn new(path: impl Into<PathBuf>, kind: TargetKind) -> Self {
        Self {
            path: path.into(),
            kind,
        }
    }

    pub fn kind(&self) -> TargetKind {
        self.kind
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MountState {
    Unmounted,
    Mounted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MountAction {
    Mount,
    None,
}

#[derive(Debug)]
pub struct PlanEntry {
    key: String,
    source: PathBuf,
    target: PathBuf,
    state: MountState,
    action: MountAction,
}

impl PlanEntry {
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn source(&self) -> &Path {
        &self.source
    }

    pub fn target(&self) -> &Path {
        &self.target
    }

    pub fn state(&self) -> MountState {
        self.state
    }

    pub fn action(&self) -> MountAction {
        self.action
    }
}

#[derive(Debug)]
pub struct Plan {
    entries: Vec<PlanEntry>,
    prune_targets: Vec<PathBuf>,
}

impl Plan {
    pub fn entries(&self) -> &[PlanEntry] {
        &self.entries
    }

    pub fn prune_targets(&self) -> &[PathBuf] {
        &self.prune_targets
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ReconcileError {
    #[error("target is a symlink: {target}")]
    SymlinkTarget { target: PathBuf },
    #[error("target is not empty: {target}")]
    NonEmptyTarget { target: PathBuf },
    #[error("unsupported target type: {target}")]
    UnsupportedTarget { target: PathBuf },
    #[error("unexpected mount at {target}")]
    UnexpectedMount { target: PathBuf },
    #[error("missing target state for {0}")]
    MissingTargetState(PathBuf),
}

pub fn reconcile(
    desired: &[DesiredMount],
    observed: &[ObservedMount],
    targets: &[TargetState],
) -> Result<Plan, ReconcileError> {
    for mount in desired {
        if let Some(nested) = observed
            .iter()
            .find(|item| item.target != mount.target && item.target.starts_with(&mount.target))
        {
            return Err(ReconcileError::UnexpectedMount {
                target: nested.target.clone(),
            });
        }
    }
    let entries = desired
        .iter()
        .map(|mount| {
            let target = target_for(targets, &mount.target)?;
            let state = match observed.iter().find(|item| item.target == mount.target) {
                Some(item) if item.source_identity == mount.source_identity => {
                    (MountState::Mounted, MountAction::None)
                }
                Some(_) => {
                    return Err(ReconcileError::UnexpectedMount {
                        target: mount.target.clone(),
                    });
                }
                None if target.kind == TargetKind::Symlink => {
                    return Err(ReconcileError::SymlinkTarget {
                        target: mount.target.clone(),
                    });
                }
                None if target.kind == TargetKind::NonEmptyDirectory => {
                    return Err(ReconcileError::NonEmptyTarget {
                        target: mount.target.clone(),
                    });
                }
                None if matches!(target.kind, TargetKind::File | TargetKind::Other) => {
                    return Err(ReconcileError::UnsupportedTarget {
                        target: mount.target.clone(),
                    });
                }
                None => (MountState::Unmounted, MountAction::Mount),
            };
            Ok(PlanEntry {
                key: mount.key.clone(),
                source: mount.source.clone(),
                target: mount.target.clone(),
                state: state.0,
                action: state.1,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut prune_targets = observed
        .iter()
        .filter(|item| !desired.iter().any(|mount| mount.target == item.target))
        .map(|item| item.target.clone())
        .collect::<Vec<_>>();
    prune_targets.sort_by_key(|target| std::cmp::Reverse(target.components().count()));
    Ok(Plan {
        entries,
        prune_targets,
    })
}

fn target_for<'a>(
    targets: &'a [TargetState],
    path: &Path,
) -> Result<&'a TargetState, ReconcileError> {
    targets
        .iter()
        .find(|target| target.path == path)
        .ok_or_else(|| ReconcileError::MissingTargetState(path.to_owned()))
}
