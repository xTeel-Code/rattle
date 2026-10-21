### Rattle ###
is newly developed project manager CLI written in Rust (WIP)
just to see your projects in one app with no looking for specific folder

## Install
```sh
cargo install --path .
```

## TUI
Run `rattle` with no arguments (or `rattle tui`) to open the interactive browser:

| Key | Action |
| --- | --- |
| `↑`/`↓`, `j`/`k`, `g`/`G` | move |
| `Enter` | quit and print the selected path |
| `/` | filter by name or path (`Esc` clears) |
| `a` | add a project folder |
| `d` | delete the entry (files on disk are kept) |
| `s` | scan the `scan_dirs` from `config.toml` |
| `e` | open the project in `$VISUAL` / `$EDITOR` |
| `c` | edit `config.toml` in your editor; reloads and rescans when you close it |
| `?` | key help |
| `q` / `Esc` | quit |

The TUI draws on stderr, so you can jump straight into a project:
```sh
cd "$(rattle)"
# or as a shell function:  rcd() { local d; d="$(rattle)" && [ -n "$d" ] && cd "$d"; }
```
Projects whose folder no longer exists are marked `missing`.

## Commands
```
rattle [command] [projectName | filePath]
help    - shows help
show    - lists your projects               (aliases: list, ls)
create  - registers a project folder        (aliases: add)      e.g. rattle create ~/code/foo
delete  - removes a project entry by name   (aliases: remove, rm) or by its number from `show`
scan    - registers every project found in your scan folders
          rattle scan [-n | --dry-run] [-d | --depth N] [folder...]
config  - opens config.toml in $VISUAL / $EDITOR  (`rattle config path` prints its path)
```
Omitted arguments are asked for interactively.

Projects are stored in `~/.rattle/projects.toml` (override with `RATTLE_FILE`).
Files written by the old Python version are read without changes.

## Scanning
`rattle scan` searches each scan folder up to `max_depth` levels deep (default 3)
and registers every folder that contains a project marker (`.git`, `Cargo.toml`,
`pyproject.toml`, `package.json`, `go.mod`, `pom.xml`, `build.gradle`,
`CMakeLists.txt`, `Makefile`).

- It stops at a project: subfolders inside a project are never registered separately.
- Hidden folders and folders like `node_modules`, `target`, `venv`, `build`,
  `dist`, `vendor` are never searched.
- Already registered paths and name clashes are skipped, so rescanning is safe.

So with `scan_dirs = ["~/code"]`, all of these are found:
```
~/code/rattle                     (depth 1)
~/code/work/api                   (depth 2)
~/code/github/xTeel-Code/rattle   (depth 3)
```

Folders passed on the command line are scanned once; without them, the
folders from `~/.rattle/config.toml` are used (created on first run).
The TUI rescans them automatically every time it opens:
```toml
scan_dirs = ["~/code", "~/work"]
max_depth = 3          # rattle scan --depth N overrides it
auto_scan = true       # rescan when the TUI opens
# optional, replace the defaults
markers = [".git", "Cargo.toml"]
ignore = ["node_modules", "target"]
```
Edit it with `c` in the TUI or `rattle config` in the shell.

## Roadmap
- [x] Made Working MVP
- [x] Rewritten in Rust
- [x] Auto Scanning for predefined folder
- [ ] Removing not only entries but whole directories (This one will be on config entry turnable)
- [ ] Creating and manipulating packages
