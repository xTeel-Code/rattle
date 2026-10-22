# Rattle

All your projects in one place, without hunting for folders. Rattle is a
terminal project manager: it finds your projects, lets you jump into one with
a keypress, and handles the chores around them.

- **Find**: scans your code folders and registers every project it finds
- **Jump**: `cd "$(rattle)"` picks a project in the TUI and takes you there
- **Open**: in your editor (`e`), or in its dev environment's browser editor (`o`)
- **Create**: new projects from templates (Rust, Python, Node, Go, or your own)
- **Dependencies**: list, add and remove them with the project's own package manager
- **Dev environments**: separate containers for groups of projects, e.g. work and personal
- **Tidy up**: drop entries whose folder is gone; optionally delete project folders

## Install
```sh
curl -fsSL https://raw.githubusercontent.com/xTeel-Code/rattle/main/install.sh | sh
```
This installs `rattle` to `~/.local/bin` on macOS (Apple Silicon or Intel) and
Linux (x86_64 or ARM). It downloads the latest release's prebuilt binary and
verifies its checksum, or builds from source with `cargo` when there is none.
The script is short; [read it](install.sh) before piping it to `sh` if you like.

Options, as environment variables in front of `sh`:

| Variable | Default | |
| --- | --- | --- |
| `RATTLE_INSTALL_DIR` | `~/.local/bin` | where to put `rattle` |
| `RATTLE_VERSION` | latest | a release tag, e.g. `v0.3.0` |
| `RATTLE_FROM_SOURCE` | `0` | `1` builds with `cargo` even when a binary exists |

```sh
curl -fsSL https://raw.githubusercontent.com/xTeel-Code/rattle/main/install.sh | RATTLE_INSTALL_DIR=/usr/local/bin sh
```
Run the same command again to update.

With Rust installed you can also use cargo directly:
```sh
cargo install --locked --git https://github.com/xTeel-Code/rattle   # latest main
cargo install --locked --path .                                     # from a clone
```

## Quick start
```sh
rattle scan ~/code         # register every project under ~/code
rattle                     # open the TUI: move with j/k, Enter jumps, ? shows all keys
cd "$(rattle)"             # pick a project and cd into it
```
To make plain `rattle scan` and the TUI's auto-scan use your folders, set
`scan_dirs` in the config: press `,` in the TUI, or run `rattle config`.

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
| `v` | dev environments: `u` start, `s` stop, `r` restart, `o`/`1`-`9` open links, `x` shell, `l` logs, `c` settings, `n` new |
| `o` | open the project in its environment's editor (starts the environment if needed) |
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
env     - dev environments: one container per group of projects (see below)
          rattle env [list | create <name> [--dir folder] | up | stop | restart |
                     logs | shell | open | config | path <name>]
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

## Dev environments
Keep groups of projects in separate containers: your work repos in one, side
projects in another, each with its own tools, git identity and storage. Projects
belong to an environment by folder (or by name), and Rattle mounts exactly those
projects into its container.

```toml
[envs.work]
dirs = ["~/code/work"]        # projects in here belong to "work"

[envs.personal]
dirs = ["~/code/side"]
projects = ["dotfiles"]       # more projects by name
# mount = false               # a plain box: code lives only inside the container
```

```sh
rattle env create work --dir ~/code/work   # also adds [envs.work] to the config
rattle env config work                     # review its settings (.env), e.g. PASSWORD
rattle env up work                         # start it; prints its links
rattle env open work --project api         # browser editor at that project
rattle env shell work api                  # a shell in the container, in that project
rattle env stop work
```
In the TUI, projects show their environment as `[work ●]` (● running, ○ stopped),
`v` manages environments and `o` opens the selected project in its environment.

How it works:
- Each environment's files live in `~/.rattle/envs/<name>/`: the template's
  `compose.yaml` and `.env`, with a unique name (`rattle-<name>`) and a free port
  range filled in, so environments never collide.
- On every start Rattle rewrites `compose.override.yaml` to bind-mount the
  environment's folders under the template's workdir (`~/code/work` →
  `/workspace/work`). Changing which folders belong to it recreates the container
  on the next start; Rattle says so first.
- It needs Docker with Compose (Docker Desktop, OrbStack or colima) and `curl`.

### Templates
The built-in template is [bindle-le](https://github.com/suckharder/bindle-le): a
browser VS Code box with Node, Python, Go, Docker and Claude Code. Every part of a
template is configurable, so you can tweak it or bring your own compose setup:

```toml
[env_templates.bindle]
compose = "https://raw.githubusercontent.com/suckharder/bindle-le/main/template/compose.yaml"
env_file = "https://raw.githubusercontent.com/suckharder/bindle-le/main/template/.env.example"
port_base = 18000            # first environment; each next one +port_step
port_step = 100

[env_templates.bindle.links] # opened by `o` / `rattle env open`; ${VAR} comes from .env
editor = "http://127.0.0.1:${PORT_CODE}/?folder=${FOLDER}"

[env_templates.mine]         # a new template: any compose project works
compose = "~/templates/dev/compose.yaml"
service = "dev"              # the service projects are mounted into
workdir = "/src"
[env_templates.mine.ports]   # .env variable = offset from the environment's base port
WEB_PORT = 0
```
Other settings: `name_var`, `shell_user`, `shell`, and at the top level
`envs_dir`, `docker` (e.g. `"podman"`) and `browser`.

### Security
- bindle runs **privileged** (it has Docker inside). A container escape is a host
  compromise, and anything in the box can change the projects mounted into it.
  Only mount what belongs there; don't use it for untrusted code.
- Without `PASSWORD` in its `.env`, bindle's editor has **no authentication**.
  Its ports bind to `127.0.0.1` only; keep it that way.
- Rattle never runs `docker compose down -v`, which deletes an environment's volumes.

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
browser = "open -a Firefox"        # opens environment links; default: open / xdg-open
docker = "docker"                  # container CLI for environments, e.g. "podman"
# optional, replace the defaults
markers = [".git", "Cargo.toml"]
ignore = ["node_modules", "target"]

[templates]                        # your own `rattle new` templates
web = "npm create vite@latest {name} -- --template react-ts"

[envs.work]                        # dev environments, see above
dirs = ["~/code/work"]
```
Everything else lives in the commented template Rattle writes on first run.
GUI editors (`zed`, `code`, `cursor`, `subl`, …) get `--wait` automatically when
editing the config, so Rattle reloads it once you close the file; projects opened
with `e` don't wait.

## Development
```sh
cargo fmt                              # format
cargo clippy --all-targets             # lint
cargo test                             # unit + end-to-end tests
cargo test -- --ignored                # also tests that need network access
sh tests/install_test.sh               # install.sh against a local fake release
```
The end-to-end tests run the real binary in a throwaway home folder, so they
never touch your `~/.rattle`. `tests/cli.rs` covers the commands, `tests/env.rs`
dev environments (with a fake `docker`), and `tests/tui.rs` drives the TUI inside
tmux (skipped when tmux isn't installed). CI runs all of this on Linux and macOS
for every pull request, plus shellcheck and a static Linux release build.

### Releasing
Bump `version` in `Cargo.toml`, then tag:
```sh
git tag v0.3.0 && git push origin v0.3.0
```
The release workflow builds binaries for macOS (arm64, x86_64) and Linux
(x86_64, arm64; static musl), uploads them with SHA-256 checksums, and publishes
the release. `install.sh` picks them up from then on.

## Roadmap
- [x] Made Working MVP
- [x] Rewritten in Rust
- [x] Auto Scanning for predefined folder
- [x] Removing not only entries but whole directories (This one will be on config entry turnable)
- [x] Creating and manipulating packages
- [x] Dev environments: separate containers for groups of projects
- [x] One-line install script
- [ ] Dev environments on another machine (over SSH)
