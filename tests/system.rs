use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use mountlist::Manifest;
use mountlist::inspect_system;

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mountlist-system-{}-{}",
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
fn system_snapshot_reads_findmnt_without_a_state_file() {
    let temp = TestDir::new();
    let source = temp.path().join("documents");
    let config = temp.path().join("config.yaml");
    fs::create_dir(&source).expect("source should be created");
    fs::write(
        &config,
        format!(
            "version: 1\nroot: {}\nshares:\n  documents: {}\n",
            temp.path().join("shared").display(),
            source.display()
        ),
    )
    .expect("config should be written");
    let manifest = Manifest::load(&config).expect("manifest should load");

    let snapshot = inspect_system(&manifest).expect("system state should load");

    assert!(snapshot.observed().is_empty());
    assert_eq!(snapshot.desired().len(), 1);
    assert_eq!(snapshot.targets()[0].kind(), mountlist::TargetKind::Missing);
}
