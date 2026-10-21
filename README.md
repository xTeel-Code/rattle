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
| `n` | create a new project from a template |
| `p` | dependencies of the selected project: `a` add, `A` add dev, `x` remove, `r` reload |
| `d` | delete the entry (files on disk are kept) |
| `D` | delete the entry **and its folder** (only if `delete_files` allows it) |
| `X` | remove entries whose folder no longer exists |
| `s` | scan the `scan_dirs` from `config.toml` |
| `e` | open the project in your editor (`editor` in config, else `$VISUAL` / `$EDITOR`) |
| `c` | edit `config.toml` in your editor; reloads and rescans when you close it |
| `,` | settings form: edit the config without leaving the TUI |
| `?` | all keys (`j`/`k` scroll in small terminals) |
| `q` / `Esc` | quit |

Templates (`n`) and package managers (`p`) run in the background with a
spinner; `Esc` cancels them, including anything they started.

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
          rattle delete <name> --files also deletes the folder (see delete_files)
scan    - registers every project found in your scan folders
          rattle scan [-n | --dry-run] [-d | --depth N] [folder...]
prune   - removes entries whose folder no longer exists   rattle prune [-n | --dry-run]
new     - creates a project from a template and registers it
          rattle new <name> [-t | --template T] [--in folder]    rattle new --list
deps    - lists, adds or removes dependencies with the project's package manager
          rattle deps [project] [list | add [--dev] <pkg>... | remove <pkg>...]
config  - opens config.toml in your editor  (`rattle config path` prints its path)
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
- Entries whose folder is gone are reported; the scan offers to remove them
  (it never does so on its own). `rattle prune` removes them directly.

So with `scan_dirs = ["~/code"]`, all of these are found:
```
~/code/rattle                     (depth 1)
~/code/work/api                   (depth 2)
~/code/github/xTeel-Code/rattle   (depth 3)
```

## New projects
`rattle new <name>` (or `n` in the TUI) runs a template in `new_project_dir`
(default: the first scan folder) and registers the result.

| Template | Command |
| --- | --- |
| `empty` | `mkdir {name} && git init -q {name}` |
| `rust` / `rust-lib` | `cargo new [--lib] {name}` |
| `python` / `python-lib` | `uv init [--lib] {name}` |
| `node` | `npm init -y` + `git init` |
| `go` | `go mod init {name}` + `git init` |

Add your own (or override these) in the config:
```toml
[templates]
web = "npm create vite@latest {name} -- --template react-ts"
```

## Dependencies
`rattle deps` (or `p` in the TUI) reads the project's manifest and uses its own
package manager to change it:

| Manifest | Manager (picked by lockfile) |
| --- | --- |
| `Cargo.toml` | `cargo add` / `cargo remove` |
| `package.json` | `npm`, `pnpm`, `yarn` or `bun` |
| `pyproject.toml` | `uv` or `poetry` |
| `go.mod` | `go get` / `go get pkg@none` |

Removing a dependency uses the group it is listed in (dev, build, optional, …).
Without a project, `rattle deps` works on the current folder.

## Deleting project folders
Off by default. Set `delete_files` to allow `D` / `rattle delete <name> --files`:

- `"off"`: only registry entries are ever removed.
- `"trash"`: the folder is moved to the Trash after a `y`.
- `"permanent"`: the folder is deleted for good after you type the project name.

Rattle refuses to delete anything outside your home folder or scan folders,
top-level folders, and folders that are or contain your home folder, a scan
folder, Rattle's own data or other registered projects.

## Config
`~/.rattle/config.toml` is created on first run. Edit it with `,` (settings form)
or `c` in the TUI, or `rattle config` in the shell:
```toml
scan_dirs = ["~/code", "~/work"]   # relative paths mean under ~
max_depth = 3                      # rattle scan --depth N overrides it
auto_scan = true                   # rescan when the TUI opens
editor = "zed"                     # overrides $VISUAL / $EDITOR
delete_files = "off"               # off | trash | permanent
new_project_dir = "~/code"         # default: the first scan folder
# optional, replace the defaults
markers = [".git", "Cargo.toml"]
ignore = ["node_modules", "target"]
```
GUI editors (`zed`, `code`, `cursor`, `subl`, …) get `--wait` automatically when
editing the config, so Rattle reloads it once you close the file; projects opened
with `e` don't wait.

## Development
```sh
cargo fmt                              # format
cargo clippy --all-targets             # lint
cargo test                             # unit + end-to-end tests
cargo test -- --ignored                # also tests that need network access
```
The end-to-end tests run the real binary in a throwaway home folder, so they
never touch your `~/.rattle`. `tests/cli.rs` covers the commands and
`tests/tui.rs` drives the TUI inside tmux (skipped when tmux isn't installed).
CI runs all of this on Linux and macOS for every pull request.

## Roadmap
- [x] Made Working MVP
- [x] Rewritten in Rust
- [x] Auto Scanning for predefined folder
- [x] Removing not only entries but whole directories (This one will be on config entry turnable)
- [x] Creating and manipulating packages
