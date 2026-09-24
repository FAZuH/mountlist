use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use mountlist::TargetKind;
use mountlist::inspect_target;
use mountlist::prepare_target;

static NEXT_TEMP_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "mountlist-filesystem-{}-{}",
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
fn target_symlink_is_detected_without_following_it() {
    let temp = TestDir::new();
    let destination = temp.path().join("destination");
    let target = temp.path().join("target");
    fs::create_dir(&destination).expect("destination should be created");
    symlink(&destination, &target).expect("symlink should be created");

    let state = inspect_target(&target).expect("target should be inspected");

    assert_eq!(state.kind(), TargetKind::Symlink);
}

#[test]
fn missing_target_is_detected() {
    let temp = TestDir::new();
    let target = temp.path().join("missing");

    let state = inspect_target(&target).expect("missing target should be inspected");

    assert_eq!(state.kind(), TargetKind::Missing);
}

#[test]
fn empty_directory_target_is_detected() {
    let temp = TestDir::new();
    let target = temp.path().join("target");
    fs::create_dir(&target).expect("target should be created");

    let state = inspect_target(&target).expect("empty target should be inspected");

    assert_eq!(state.kind(), TargetKind::EmptyDirectory);
}

#[test]
fn non_empty_directory_target_is_detected() {
    let temp = TestDir::new();
    let target = temp.path().join("target");
    fs::create_dir(&target).expect("target should be created");
    fs::write(target.join("existing"), b"data").expect("existing file should be written");

    let state = inspect_target(&target).expect("non-empty target should be inspected");

    assert_eq!(state.kind(), TargetKind::NonEmptyDirectory);
}

#[test]
fn preparing_target_creates_only_the_target_and_preserves_source() {
    let temp = TestDir::new();
    let source = temp.path().join("source");
    let target = temp.path().join("shared/target");
    fs::create_dir(&source).expect("source should be created");
    fs::write(source.join("data"), b"unchanged").expect("source file should be written");

    prepare_target(&source, &target).expect("target should be prepared");

    assert_eq!(fs::read(source.join("data")).unwrap(), b"unchanged");
    assert_eq!(
        inspect_target(&target).unwrap().kind(),
        TargetKind::EmptyDirectory
    );
}
