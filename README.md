<div align="center">

# mountlist

**Reconcile local bind mounts from one YAML manifest.**

</div>

<hr>

<div align="center">
● <a href="#installation">Installation</a> ﻿ ● <a href="#usage">Usage</a> ﻿ ● <a href="#docs">Docs</a> ﻿ ● <a href="#license">License</a>
</div>

## Installation

```sh
cargo install --path .
```

`mountlist` uses Linux `findmnt`, `mount`, and `umount` commands.

## Usage

Copy [the example manifest](config.example.yaml) to `~/.config/shared-mounts/config.yaml`, then edit its root and source paths.

```sh
mountlist plan
mountlist status
sudo mountlist apply
sudo mountlist prune --yes
```

`plan` and `status` do not require root. `apply` creates missing target paths and bind mounts each configured source. `prune --yes` only unmounts mount points under the shared root that are no longer in the manifest. It never deletes target directories or changes source files.

Use `--config PATH` to select another manifest.

## Docs

- [Architecture](docs/architecture.md) - module map, runtime flow, and safety model
- [Example manifest](config.example.yaml) - generic root and share configuration
- CLI reference - run `mountlist help`

## License

MIT
