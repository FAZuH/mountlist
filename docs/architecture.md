# Architecture

## Purpose and scope

`mountlist` reconciles local Linux bind mounts from one YAML manifest. A share is a source directory or file that appears under a shared root. The tool compares the manifest with the current Linux mount table. It then reports, mounts, or removes bind mounts to match the manifest.

`mountlist` handles only these operations:

- It reads local paths from a version 1 YAML manifest.
- It identifies the filesystem object behind each source path.
- It reports the current state and the planned actions.
- It creates a target path and runs a bind mount for each missing share.
- It unmounts stale mount points under the shared root.

The tool does not manage source contents, target directory contents, or a persistent state file. The Linux mount table is the current system state.

## Module map

The crate has one binary and one library. The binary handles the command line. The library holds the data model, planner, and operating system operations.

### `src/main.rs`

`main.rs` defines the command-line interface and coordinates one command. The global `--config` option selects a manifest path. The command is one of `plan`, `status`, `apply`, or `prune`.

The module also defines the default manifest path. It uses `$HOME/.config/shared-mounts/config.yaml`. `apply` and `prune` call the root check before they load the manifest. The module prints plans and runs the action selected by the command.

### `src/config.rs`

`config.rs` reads and validates the manifest. It rejects unknown fields, unsupported versions, and duplicate share keys. It derives each target path from the share key and the shared root.

This module also checks that sources are absolute, existing, and outside the shared root. It requires the share key to exactly match the source basename. The loaded `Manifest` contains the optional device metadata, the shared root, and an ordered list of `Share` values.

### `src/lib.rs`

`lib.rs` contains the domain model and the pure reconciliation logic. It defines desired mounts, observed mounts, target states, mount states, and mount actions. It also defines the reconciliation plan and reconciliation errors.

The `reconcile` function compares desired data with observed data. It does not run operating system commands. This separation keeps the planner testable without a real mount table.

### `src/os.rs`

`os.rs` contains the Linux seam. It calls `findmnt` to inspect the mount table, uses the filesystem to inspect target paths, and runs `mount` and `umount` for changes. It also checks the effective user ID in `/proc/self/status`.

The module converts filesystem and command results into typed errors. The library exposes its inspection and action functions to the binary.

The class diagram for the configuration and data model is at [`docs/diagrams/config-model.mmd`](diagrams/config-model.mmd). The class diagram for reconciliation is at [`docs/diagrams/reconciliation-model.mmd`](diagrams/reconciliation-model.mmd).

## Runtime flow

All commands follow the same inspection flow:

1. `main.rs` parses the command line with `clap`.
2. For `apply` and `prune`, `main.rs` requires effective user ID 0.
3. The command selects `--config` or the default path under `$HOME`.
4. `Manifest::load` reads and validates the YAML manifest.
5. `inspect_system` reads the current system and builds a `SystemSnapshot`.
6. `reconcile` compares the snapshot with the manifest and returns a `Plan` or an error.
7. The command prints the plan or performs its selected actions.

The command behavior differs after the plan is built:

- `plan` prints each desired share and labels a missing mount as `mount`. It labels a matching mount as `mounted`.
- `status` prints the observed state. It labels a desired mount as `mounted` or `missing`.
- `apply` calls `prepare_target` and `mount_bind` for every entry whose action is `Mount`.
- `prune` calls `umount` for every stale target in the plan.

`apply` and `prune` inspect the complete system before they change it. A reconciliation error stops the command before any action runs.

## Configuration and system snapshot

A manifest has this shape:

```yaml
version: 1
device: generic-host
root: /srv/shared
shares:
  documents: /srv/data/documents
  photos: /srv/data/photos
```

The `version` field must equal 1. The `device` field is optional manifest metadata. The current source identity does not use this field. It uses the device identifier returned by `findmnt`.

The `root` field defines the common parent for target paths. The `shares` field maps each exact share key to one absolute source path. For example, the key `documents` creates the target `/srv/shared/documents`.

`Manifest::load` performs these checks:

- It rejects relative root and source paths.
- It rejects `/` as the shared root.
- It rejects a shared root that contains `.` or `..` components.
- It rejects a shared root that is a symlink or a non-directory.
- It accepts a shared root that does not exist yet.
- It rejects a missing source.
- It rejects a source inside the shared root, including through a resolved path.
- It requires each share key to be one exact path component.
- It requires the key to equal the source basename.

The `BTreeMap` used for shares gives the loaded shares a stable order. The custom deserializer rejects duplicate keys because duplicate keys would create duplicate target paths.

`inspect_system` creates three collections:

- `desired`: one `DesiredMount` for each manifest share.
- `observed`: mount points found under the shared root, excluding the root itself.
- `targets`: one `TargetState` for each desired target.

The snapshot does not include a stored mount state. Every run reads the current Linux mount table.

## Linux operating system seam

### Inspecting mount points

`inspect_system` calls `findmnt --json --list --output TARGET,MAJ:MIN,FSROOT`. It keeps mount points whose paths are below the manifest root. It excludes the root itself.

For each source, `inspect_system` calls `findmnt --json --evaluate --target SOURCE --output TARGET,MAJ:MIN,FSROOT`. It combines the returned device identifier and filesystem root with the source path. The result is a `MountIdentity`.

A `MountIdentity` has two fields:

- The device identifier, such as the `MAJ:MIN` value.
- The path from the containing filesystem root to the source.

This identity allows the planner to detect a matching source even when the manifest path is a symlink or uses a different spelling for the same path. The manifest source is canonicalized before the identity is created.

### Inspecting targets

`inspect_target` uses `symlink_metadata` so it does not follow a target symlink. It reports one of these target kinds:

- `Missing`
- `EmptyDirectory`
- `NonEmptyDirectory`
- `File`
- `Symlink`
- `Other`

A directory is empty when `read_dir` returns no entries. Any other existing directory is non-empty. The shared root can exist later, but each target may be absent when the system is inspected.

### Mounting a share

`apply` calls `prepare_target` for each missing share. `prepare_target` reads the source metadata. It then creates the target parent directories. For a directory source, it creates an empty target directory. For a file source, it creates a new empty target file with `create_new(true)`. It does not overwrite an existing file.

`prepare_target` returns an error when the source is neither a directory nor a regular file. It also returns an error when an existing directory is not empty.

`mount_bind` runs:

```text
mount --bind -- SOURCE TARGET
```

The `--` option ends command options before the path arguments. A non-zero command status becomes a `MountError::Command` with the command, status, and standard error text.

### Removing a stale mount

`prune` calls `umount -- TARGET` for each stale target. It unmounts stale targets in deepest-first order. A non-zero command status becomes a `MountError::Command`.

`prune` does not delete target directories. It does not change source files. The `--yes` option is required by the CLI before this action runs.

## Domain model and reconciliation

### Core entities

- `Manifest`: validated configuration for a shared root and its shares.
- `Share`: one manifest key, source path, and derived target path.
- `DesiredMount`: a share that should exist at a target.
- `ObservedMount`: a mount point found under the shared root.
- `MountIdentity`: the device and filesystem path that identify a source.
- `TargetState`: a target path and its current filesystem kind.
- `SystemSnapshot`: desired mounts, observed mounts, and target states at one inspection.
- `PlanEntry`: one desired mount, its state, and its action.
- `Plan`: desired entries plus stale targets for pruning.

### Mount states and actions

A desired target has one of two states:

- `Unmounted`: no observed mount exists at the target.
- `Mounted`: an observed mount exists at the target with the expected source identity.

A plan entry has one of two actions:

- `Mount`: the target is unmounted and safe to prepare for a bind mount.
- `None`: the expected mount already exists.

There is no action for unmounting a desired target. A mount at a desired target with a different identity is an error. This prevents `mountlist` from replacing a mount that it does not understand.

### Reconciliation rules

`reconcile` first rejects an observed mount below any desired target. This nested mount check runs before it creates plan entries. A nested mount can make a parent mount difficult to reason about, so the command stops instead of changing it.

For each desired mount, the planner finds the matching `TargetState`:

- A matching observed mount with the same `MountIdentity` produces `Mounted` and `None`.
- A different observed mount at the target produces `UnexpectedMount`.
- An unmounted symlink target produces `SymlinkTarget`.
- An unmounted non-empty directory produces `NonEmptyTarget`.
- An unmounted file or other filesystem object produces `UnsupportedTarget`.
- A missing or empty-directory target produces `Unmounted` and `Mount`.

The planner does not reject a non-empty directory when the expected mount already exists. The target contents belong to the mounted source in that state. A matching mount therefore remains idempotent.

The planner finds observed mounts that are under the shared root but are not desired target paths. It puts their paths in `prune_targets`. It sorts this list by path depth in reverse order. A nested stale mount is therefore unmounted before its parent path.

The reconciliation model is shown in [`docs/diagrams/reconciliation-model.mmd`](diagrams/reconciliation-model.mmd).

## Safety invariants and privilege model

`mountlist` uses these safety invariants:

- The manifest version is 1.
- The shared root is not `/` and does not contain `.` or `..` path components.
- The shared root is not a symlink and, when it exists, is a directory.
- Every source is absolute, exists at load time, and is outside the shared root.
- Every share key is one exact path component and matches the source basename.
- A target is never treated as a symlink.
- An unmounted target must be absent or an empty directory before `apply` creates or reuses it.
- `apply` never overwrites an existing file target.
- A mount at a desired target must have the expected source identity.
- A nested mount below a desired target stops reconciliation.
- `prune` touches only observed mount points below the shared root that are not desired target paths.
- `prune` unmounts stale nested targets before parent targets.
- `prune` does not delete target directories or source data.

`apply` and `prune` require effective user ID 0. `require_root` reads `/proc/self/status` and checks the second number in the `Uid:` line. The program does not call `sudo`. A caller must start these commands with the required privileges. `plan` and `status` do not call the root check because they only inspect and print.

The code uses path arguments as separate process arguments. It does not build a shell command string. The Linux commands still control the final mount and unmount behavior.

## Tests and filesystem integration

The test suite separates planning, manifest validation, filesystem target handling, and system inspection.

- `tests/planner.rs` tests pure reconciliation with in-memory desired mounts, observed mounts, and target states. It covers matching mounts, missing mounts, unexpected sources, nested mounts, stale mounts, prune order, symlink targets, non-empty targets, and unsupported target types. It does not invoke Linux commands.
- `tests/manifest.rs` writes temporary YAML manifests and creates temporary source and root paths. It covers target derivation, optional device metadata, version checks, path checks, missing sources, source placement, key shape, duplicate keys, filesystem root rejection, and symlink root rejection. It uses a `Drop` helper to remove its temporary directory.
- `tests/filesystem.rs` uses real temporary directories and a Unix symlink. It tests target inspection without following symlinks, missing, empty, and non-empty targets, and target preparation. The preparation test confirms that source data remains unchanged.
- `tests/system.rs` uses a real temporary manifest and calls `inspect_system`. It calls the installed `findmnt` command, checks that no stale mount is observed under the temporary root, and checks the desired mount and target state. It does not mount or unmount anything.

The tests use `std::env::temp_dir()` and process-specific counters to keep test paths separate. The system test is the main integration boundary for `findmnt`. The mount and unmount command wrappers are not exercised by the current tests because those actions require privileged system changes.

## Extension points

The current boundaries provide a small set of extension points:

- Add manifest rules in `config.rs` and preserve the `Manifest` and `Share` data model.
- Add reconciliation rules in `lib.rs` and test them with in-memory entities.
- Add Linux inspection or command behavior in `os.rs` behind the existing library functions.
- Add a command in `main.rs` and route it through manifest inspection and reconciliation when it needs the current system state.

The source currently uses direct function calls instead of a trait for operating system commands. A new platform would require replacing or wrapping the `os.rs` boundary. It would also require a new design for target and mount behavior.

## Non-goals

`mountlist` does not provide these features:

- It does not persist a state database or cache mount state between runs.
- It does not use the optional manifest `device` field to choose a device or a source.
- It does not manage source directories or source files.
- It does not delete target directories after unmounting.
- It does not unmount a desired mount that has the wrong source identity.
- It does not replace a mount that is not in the manifest. The `prune` command handles only observed stale mounts under the shared root.
- It does not add mount options such as read-only, propagation, or user mapping.
- It does not call `sudo`, change privileges, or provide a privilege escalation flow.
- It does not support non-Linux operating systems in the current implementation.
- It does not persist a manifest or edit `config.yaml`.
