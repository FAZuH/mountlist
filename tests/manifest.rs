use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use mountlist::Manifest;
use mountlist::ManifestError;

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mountlist-manifest-{}-{}",
            std::process::id(),
            NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("test directory should be created");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn load_derives_each_target_from_its_exact_share_key() {
    let temp = TestDir::new();
    let source = temp.path().join("documents");
    let root = temp.path().join("shared");
    fs::create_dir(&source).expect("source should be created");
    let config = temp.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            "version: 1\nroot: {}\nshares:\n  documents: {}\n",
            root.display(),
            source.display()
        ),
    )
    .expect("config should be written");

    let manifest = Manifest::load(&config).expect("manifest should load");

    assert_eq!(manifest.root(), root.as_path());
    assert_eq!(manifest.shares()[0].key(), "documents");
    assert_eq!(manifest.shares()[0].source(), source.as_path());
    assert_eq!(
        manifest.shares()[0].target(),
        root.join("documents").as_path()
    );
}

#[test]
fn load_accepts_version_one_and_optional_device_metadata() {
    let temp = TestDir::new();
    let source = temp.path().join("documents");
    fs::create_dir(&source).expect("source should be created");
    let config = temp.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            "version: 1\ndevice: example-device\nroot: {}\nshares:\n  documents: {}\n",
            temp.path().join("shared").display(),
            source.display()
        ),
    )
    .expect("config should be written");

    let manifest = Manifest::load(&config).expect("manifest metadata should load");

    assert_eq!(manifest.device(), Some("example-device"));
}

#[test]
fn load_rejects_unsupported_manifest_version() {
    let temp = TestDir::new();
    let source = temp.path().join("documents");
    fs::create_dir(&source).expect("source should be created");
    let config = temp.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            "version: 2\nroot: {}\nshares:\n  documents: {}\n",
            temp.path().join("shared").display(),
            source.display()
        ),
    )
    .expect("config should be written");

    let error = Manifest::load(&config).expect_err("unsupported version should fail");

    assert!(matches!(
        error,
        ManifestError::UnsupportedVersion { version: 2 }
    ));
}

#[test]
fn load_rejects_case_mismatched_share_key_and_source_basename() {
    let temp = TestDir::new();
    let source = temp.path().join("documents");
    fs::create_dir(&source).expect("source should be created");
    let config = temp.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            "version: 1\nroot: {}\nshares:\n  Documents: {}\n",
            temp.path().join("shared").display(),
            source.display()
        ),
    )
    .expect("config should be written");

    let error = Manifest::load(&config).expect_err("basename mismatch should fail");

    assert!(matches!(error, ManifestError::BasenameMismatch { .. }));
}

#[test]
fn load_rejects_relative_source_path() {
    let temp = TestDir::new();
    let config = temp.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            "version: 1\nroot: {}\nshares:\n  documents: documents\n",
            temp.path().join("shared").display()
        ),
    )
    .expect("config should be written");

    let error = Manifest::load(&config).expect_err("relative source should fail");

    assert!(matches!(error, ManifestError::RelativePath { .. }));
}

#[test]
fn load_rejects_relative_root_path() {
    let temp = TestDir::new();
    let source = temp.path().join("documents");
    fs::create_dir(&source).expect("source should be created");
    let config = temp.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            "version: 1\nroot: shared\nshares:\n  documents: {}\n",
            source.display()
        ),
    )
    .expect("config should be written");

    let error = Manifest::load(&config).expect_err("relative root should fail");

    assert!(matches!(error, ManifestError::RelativePath { .. }));
}

#[test]
fn load_rejects_missing_source() {
    let temp = TestDir::new();
    let source = temp.path().join("documents");
    let config = temp.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            "version: 1\nroot: {}\nshares:\n  documents: {}\n",
            temp.path().join("shared").display(),
            source.display()
        ),
    )
    .expect("config should be written");

    let error = Manifest::load(&config).expect_err("missing source should fail");

    assert!(matches!(error, ManifestError::MissingSource { .. }));
}

#[test]
fn load_rejects_source_inside_shared_root() {
    let temp = TestDir::new();
    let root = temp.path().join("shared");
    let source = root.join("documents");
    fs::create_dir_all(&source).expect("source should be created");
    let config = temp.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            "version: 1\nroot: {}\nshares:\n  documents: {}\n",
            root.display(),
            source.display()
        ),
    )
    .expect("config should be written");

    let error = Manifest::load(&config).expect_err("source inside root should fail");

    assert!(matches!(error, ManifestError::SourceInsideRoot { .. }));
}

#[test]
fn load_rejects_share_key_that_is_not_one_exact_path_component() {
    let temp = TestDir::new();
    let source = temp.path().join("documents");
    fs::create_dir(&source).expect("source should be created");
    let config = temp.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            "version: 1\nroot: {}\nshares:\n  nested/documents: {}\n",
            temp.path().join("shared").display(),
            source.display()
        ),
    )
    .expect("config should be written");

    let error = Manifest::load(&config).expect_err("nested share key should fail");

    assert!(matches!(error, ManifestError::InvalidShareKey { .. }));
}

#[test]
fn load_rejects_duplicate_share_keys_and_targets() {
    let temp = TestDir::new();
    let source = temp.path().join("documents");
    fs::create_dir(&source).expect("source should be created");
    let config = temp.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            "version: 1\nroot: {}\nshares:\n  documents: {}\n  documents: {}\n",
            temp.path().join("shared").display(),
            source.display(),
            source.display()
        ),
    )
    .expect("config should be written");

    let error = Manifest::load(&config).expect_err("duplicate target should fail");

    assert!(matches!(error, ManifestError::Parse(_)));
}

#[test]
fn load_rejects_filesystem_root_as_shared_root() {
    let temp = TestDir::new();
    let source = temp.path().join("documents");
    fs::create_dir(&source).expect("source should be created");
    let config = temp.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            "version: 1\nroot: /\nshares:\n  documents: {}\n",
            source.display()
        ),
    )
    .expect("config should be written");

    let error = Manifest::load(&config).expect_err("filesystem root should fail");

    assert!(matches!(error, ManifestError::UnsafeRoot { .. }));
}

#[test]
fn load_rejects_symlink_shared_root() {
    let temp = TestDir::new();
    let root = temp.path().join("root-target");
    let linked_root = temp.path().join("shared");
    let source = temp.path().join("documents");
    fs::create_dir(&root).expect("root target should be created");
    fs::create_dir(&source).expect("source should be created");
    symlink(&root, &linked_root).expect("root symlink should be created");
    let config = temp.path().join("config.yaml");
    fs::write(
        &config,
        format!(
            "version: 1\nroot: {}\nshares:\n  documents: {}\n",
            linked_root.display(),
            source.display()
        ),
    )
    .expect("config should be written");

    let error = Manifest::load(&config).expect_err("symlink root should fail");

    assert!(matches!(error, ManifestError::UnsafeRoot { .. }));
}
