use std::path::PathBuf;

use mountlist::DesiredMount;
use mountlist::MountAction;
use mountlist::MountIdentity;
use mountlist::MountState;
use mountlist::ObservedMount;
use mountlist::ReconcileError;
use mountlist::TargetKind;
use mountlist::TargetState;
use mountlist::reconcile;

#[test]
fn unmounted_empty_target_is_planned_for_mounting() {
    let identity = MountIdentity::new("0:42", "/source");
    let desired = DesiredMount::new(
        "documents",
        "/srv/documents",
        "/mnt/shared/documents",
        identity,
    );
    let target = TargetState::new("/mnt/shared/documents", TargetKind::Missing);

    let plan = reconcile(&[desired], &[], &[target]).expect("valid target should reconcile");

    assert_eq!(plan.entries().len(), 1);
    assert_eq!(plan.entries()[0].state(), MountState::Unmounted);
    assert_eq!(plan.entries()[0].action(), MountAction::Mount);
}

#[test]
fn mount_at_desired_target_with_different_source_is_rejected() {
    let desired = DesiredMount::new(
        "documents",
        "/srv/documents",
        "/mnt/shared/documents",
        MountIdentity::new("0:42", "/srv/documents"),
    );
    let observed = vec![ObservedMount::new(
        "/mnt/shared/documents",
        MountIdentity::new("0:42", "/somewhere-else"),
    )];
    let target = TargetState::new("/mnt/shared/documents", TargetKind::Missing);

    let error = reconcile(&[desired], &observed, &[target]).expect_err("wrong source must fail");

    assert_eq!(
        error,
        ReconcileError::UnexpectedMount {
            target: "/mnt/shared/documents".into()
        }
    );
}

#[test]
fn nested_mount_below_desired_target_is_rejected() {
    let desired = DesiredMount::new(
        "documents",
        "/srv/documents",
        "/mnt/shared/documents",
        MountIdentity::new("0:42", "/srv/documents"),
    );
    let observed = vec![ObservedMount::new(
        "/mnt/shared/documents/nested",
        MountIdentity::new("0:42", "/srv/documents/nested"),
    )];
    let target = TargetState::new("/mnt/shared/documents", TargetKind::NonEmptyDirectory);

    let error = reconcile(&[desired], &observed, &[target]).expect_err("nested mount must fail");

    assert_eq!(
        error,
        ReconcileError::UnexpectedMount {
            target: "/mnt/shared/documents/nested".into()
        }
    );
}

#[test]
fn mount_missing_from_manifest_is_planned_for_pruning() {
    let desired = DesiredMount::new(
        "documents",
        "/srv/documents",
        "/mnt/shared/documents",
        MountIdentity::new("0:42", "/srv/documents"),
    );
    let observed = vec![ObservedMount::new(
        "/mnt/shared/retired",
        MountIdentity::new("0:42", "/srv/retired"),
    )];
    let target = TargetState::new("/mnt/shared/documents", TargetKind::Missing);

    let plan = reconcile(&[desired], &observed, &[target]).expect("stale mount should be prunable");

    assert_eq!(plan.prune_targets(), [PathBuf::from("/mnt/shared/retired")]);
}

#[test]
fn nested_stale_mounts_are_pruned_deepest_first() {
    let desired = DesiredMount::new(
        "documents",
        "/srv/documents",
        "/mnt/shared/documents",
        MountIdentity::new("0:42", "/srv/documents"),
    );
    let observed = vec![
        ObservedMount::new(
            "/mnt/shared/retired",
            MountIdentity::new("0:42", "/srv/retired"),
        ),
        ObservedMount::new(
            "/mnt/shared/retired/nested",
            MountIdentity::new("0:42", "/srv/retired/nested"),
        ),
    ];
    let target = TargetState::new("/mnt/shared/documents", TargetKind::Missing);

    let plan =
        reconcile(&[desired], &observed, &[target]).expect("stale mounts should be prunable");

    assert_eq!(
        plan.prune_targets(),
        [
            PathBuf::from("/mnt/shared/retired/nested"),
            PathBuf::from("/mnt/shared/retired"),
        ]
    );
}

#[test]
fn symlink_target_is_rejected() {
    let desired = DesiredMount::new(
        "documents",
        "/srv/documents",
        "/mnt/shared/documents",
        MountIdentity::new("0:42", "/srv/documents"),
    );
    let target = TargetState::new("/mnt/shared/documents", TargetKind::Symlink);

    let error = reconcile(&[desired], &[], &[target]).expect_err("symlink target must fail");

    assert_eq!(
        error,
        ReconcileError::SymlinkTarget {
            target: "/mnt/shared/documents".into()
        }
    );
}

#[test]
fn non_empty_unmounted_target_is_rejected() {
    let desired = DesiredMount::new(
        "documents",
        "/srv/documents",
        "/mnt/shared/documents",
        MountIdentity::new("0:42", "/srv/documents"),
    );
    let target = TargetState::new("/mnt/shared/documents", TargetKind::NonEmptyDirectory);

    let error = reconcile(&[desired], &[], &[target]).expect_err("non-empty target must fail");

    assert_eq!(
        error,
        ReconcileError::NonEmptyTarget {
            target: "/mnt/shared/documents".into()
        }
    );
}

#[test]
fn matching_mount_is_idempotent_even_when_target_contents_are_non_empty() {
    let identity = MountIdentity::new("0:42", "/srv/documents");
    let desired = DesiredMount::new(
        "documents",
        "/srv/documents",
        "/mnt/shared/documents",
        identity.clone(),
    );
    let observed = ObservedMount::new("/mnt/shared/documents", identity);
    let target = TargetState::new("/mnt/shared/documents", TargetKind::NonEmptyDirectory);

    let plan =
        reconcile(&[desired], &[observed], &[target]).expect("matching mount should reconcile");

    assert_eq!(plan.entries()[0].state(), MountState::Mounted);
    assert_eq!(plan.entries()[0].action(), MountAction::None);
}

#[test]
fn unsupported_target_type_is_rejected() {
    let desired = DesiredMount::new(
        "documents",
        "/srv/documents",
        "/mnt/shared/documents",
        MountIdentity::new("0:42", "/srv/documents"),
    );
    let target = TargetState::new("/mnt/shared/documents", TargetKind::Other);

    let error = reconcile(&[desired], &[], &[target]).expect_err("unsupported target must fail");

    assert_eq!(
        error,
        ReconcileError::UnsupportedTarget {
            target: "/mnt/shared/documents".into()
        }
    );
}
