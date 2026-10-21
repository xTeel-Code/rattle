### Rattle ###
is newly developed project manager CLI written in Rust (WIP)
just to see your projects in one app with no looking for specific folder

## Install
```sh
cargo install --path .
```

## Usage
```
rattle [command] [projectName | filePath]
help    - shows help
show    - lists your projects               (aliases: list, ls)
create  - registers a project folder        (aliases: add)      e.g. rattle create ~/code/foo
delete  - removes a project entry by name   (aliases: remove, rm) or by its number from `show`
scan    - registers every project found in your scan folders
          rattle scan [-n | --dry-run] [folder...]
```
Omitted arguments are asked for interactively.

Projects are stored in `~/.rattle/projects.toml` (override with `RATTLE_FILE`).
Files written by the old Python version are read without changes.

## Scanning
`rattle scan` looks at the direct subfolders of each scan folder and registers
those that contain a project marker (`.git`, `Cargo.toml`, `pyproject.toml`,
`package.json`, `go.mod`, `pom.xml`, `build.gradle`, `CMakeLists.txt`, `Makefile`).
Hidden folders, already registered paths and name clashes are skipped.

Folders passed on the command line are scanned once; without them, the
folders from `~/.rattle/config.toml` are used (created on first run):
```toml
scan_dirs = ["~/code", "~/work"]
# optional, replaces the default markers
markers = [".git", "Cargo.toml"]
```

## Roadmap
- [x] Made Working MVP
- [x] Rewritten in Rust
- [x] Auto Scanning for predefined folder
- [ ] Removing not only entries but whole directories (This one will be on config entry turnable)
- [ ] Creating and manipulating packages
