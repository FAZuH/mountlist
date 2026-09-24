use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Deserializer;
use serde::de::MapAccess;
use serde::de::Visitor;
use serde::de::{self};
use thiserror::Error;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    version: u64,
    device: Option<String>,
    root: PathBuf,
    shares: Shares,
}

#[derive(Debug)]
struct Shares(BTreeMap<String, PathBuf>);

impl<'de> Deserialize<'de> for Shares {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SharesVisitor;

        impl<'de> Visitor<'de> for SharesVisitor {
            type Value = Shares;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a mapping of share keys to source paths")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut shares = BTreeMap::new();
                while let Some((key, source)) = map.next_entry::<String, PathBuf>()? {
                    if shares.insert(key.clone(), source).is_some() {
                        return Err(de::Error::custom(format!(
                            "duplicate share key would create a duplicate target: {key}"
                        )));
                    }
                }
                Ok(Shares(shares))
            }
        }

        deserializer.deserialize_map(SharesVisitor)
    }
}

#[derive(Debug)]
pub struct Share {
    key: String,
    source: PathBuf,
    target: PathBuf,
}

impl Share {
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn source(&self) -> &Path {
        &self.source
    }

    pub fn target(&self) -> &Path {
        &self.target
    }
}

#[derive(Debug)]
pub struct Manifest {
    device: Option<String>,
    root: PathBuf,
    shares: Vec<Share>,
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        let contents = fs::read_to_string(path).map_err(ManifestError::Read)?;
        let raw: RawManifest = serde_yml::from_str(&contents).map_err(ManifestError::Parse)?;
        if raw.version != 1 {
            return Err(ManifestError::UnsupportedVersion {
                version: raw.version,
            });
        }
        if !raw.root.is_absolute() {
            return Err(ManifestError::RelativePath { path: raw.root });
        }
        let root_has_parent = raw
            .root
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir));
        if raw.root == Path::new("/") || root_has_parent {
            return Err(ManifestError::UnsafeRoot { path: raw.root });
        }
        match fs::symlink_metadata(&raw.root) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(ManifestError::UnsafeRoot { path: raw.root });
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(ManifestError::InspectRoot {
                    path: raw.root,
                    error,
                });
            }
        }
        let shares = raw
            .shares
            .0
            .into_iter()
            .map(|(key, source)| {
                let mut key_components = Path::new(&key).components();
                let valid_key = matches!(
                    key_components.next(),
                    Some(Component::Normal(name)) if name == OsStr::new(&key)
                ) && key_components.next().is_none();
                if !valid_key {
                    return Err(ManifestError::InvalidShareKey { key });
                }
                if !source.is_absolute() {
                    return Err(ManifestError::RelativePath { path: source });
                }
                match fs::metadata(&source) {
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        return Err(ManifestError::MissingSource { path: source });
                    }
                    Err(error) => {
                        return Err(ManifestError::InspectSource {
                            path: source,
                            error,
                        });
                    }
                }
                let canonical_source =
                    fs::canonicalize(&source).map_err(|error| ManifestError::InspectSource {
                        path: source.clone(),
                        error,
                    })?;
                let comparison_root = match fs::canonicalize(&raw.root) {
                    Ok(root) => root,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => raw.root.clone(),
                    Err(error) => {
                        return Err(ManifestError::InspectRoot {
                            path: raw.root.clone(),
                            error,
                        });
                    }
                };
                if canonical_source.starts_with(&comparison_root) {
                    return Err(ManifestError::SourceInsideRoot {
                        source_path: source,
                    });
                }
                if source.file_name().and_then(|name| name.to_str()) != Some(key.as_str()) {
                    return Err(ManifestError::BasenameMismatch {
                        key,
                        source_path: source,
                    });
                }
                Ok(Share {
                    target: raw.root.join(&key),
                    key,
                    source,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            device: raw.device,
            root: raw.root,
            shares,
        })
    }

    pub fn device(&self) -> Option<&str> {
        self.device.as_deref()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn shares(&self) -> &[Share] {
        &self.shares
    }
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("failed to read manifest: {0}")]
    Read(std::io::Error),
    #[error("failed to parse manifest: {0}")]
    Parse(serde_yml::Error),
    #[error("unsupported manifest version {version}; expected version 1")]
    UnsupportedVersion { version: u64 },
    #[error("path must be absolute: {}", .path.display())]
    RelativePath { path: PathBuf },
    #[error("shared root must not be the filesystem root: {}", .path.display())]
    UnsafeRoot { path: PathBuf },
    #[error("source does not exist: {}", .path.display())]
    MissingSource { path: PathBuf },
    #[error("failed to inspect source {}: {error}", .path.display())]
    InspectSource {
        path: PathBuf,
        #[source]
        error: std::io::Error,
    },
    #[error("share key must be one exact path component: {key}")]
    InvalidShareKey { key: String },
    #[error("source is inside the shared root: {}", .source_path.display())]
    SourceInsideRoot { source_path: PathBuf },
    #[error("failed to inspect root {}: {error}", .path.display())]
    InspectRoot {
        path: PathBuf,
        #[source]
        error: std::io::Error,
    },
    #[error("share key {key} does not exactly match its source basename")]
    BasenameMismatch { key: String, source_path: PathBuf },
}
