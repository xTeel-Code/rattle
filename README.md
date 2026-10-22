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
Run `rattle` with no arguments (or `rattle tui`) to open the interactive browser.
It has two tabs, **Projects** and **Environments**: `Tab` (or `v`) switches
between them, `?` lists every key, `q` quits.

**Projects tab**

| Key | Action |
| --- | --- |
| `↑`/`↓`, `j`/`k`, `g`/`G` | move |
| `Enter` | quit and print the selected path |
| `/` | filter by name or path (`Esc` clears) |
| `a` | add a project folder |
| `n` | create a new project from a template |
| `p` | dependencies of the selected project: `a` add, `A` add dev, `x` remove, `r` reload |
| `o` | open the project in its environment's editor (starts the environment if needed) |
| `e` | open the project in your editor (`editor` in config, else `$VISUAL` / `$EDITOR`) |
| `d` | delete the entry (files on disk are kept) |
| `D` | delete the entry **and its folder** (only if `delete_files` allows it) |
| `X` | remove entries whose folder no longer exists |
| `s` | scan the `scan_dirs` from `config.toml` |
| `c` | edit `config.toml` in your editor; reloads and rescans when you close it |
| `,` | settings form: edit the config without leaving the TUI |

Projects in an environment show it as a tag, `[work ●]` (● running, ○ stopped).

**Environments tab**: the list on the left, details of the selected one on the
right (status, folders, mounting, member projects, links, files).

| Key | Action |
| --- | --- |
| `u` / `s` / `r` | start / stop / restart |
| `o`, `1`-`9` | open its first / numbered link (starts it if needed) |
| `x` | a shell inside it |
| `l` | its logs, in the detail pane (`j`/`k` scroll) |
| `c` | edit its settings (`.env`) |
| `n` | create a new environment (name, then folders) |
| `d` | change its project folders |
| `m` | turn mounting its projects on or off |
| `D` | remove it, after typing its name: container and files go, volumes stay |
| `Esc` | back to Projects |

Slow work runs in the background with a spinner, so the TUI never freezes.
Templates (`n`), package managers (`p`) and Docker actions can be cancelled with
`Esc`, including anything they started; a command that won't stop is killed
after 3 seconds, or at once on a second `Esc`. Deleting a folder and creating an
environment finish on their own, since stopping them halfway would leave a mess.

The TUI draws on stderr, so you can jump straight into a project:
```sh
cd "$(rattle)"
# or as a shell function:  rcd() { local d; d="$(rattle)" && [ -n "$d" ] && cd "$d"; }
```
Projects whose folder no longer exists are marked `missing`.

## Commands
The same list as `rattle help`:
```
Projects
  show                       list projects                  (aliases: list, ls)
  create [path]              register a folder, e.g. `create .`   (alias: add)
  delete <name | number>     remove an entry; files stay  (aliases: remove, rm)
    --files                  also delete the folder (see delete_files in config)
  scan [folder...]           register projects in folders (default: scan_dirs)
    -n, --dry-run            only show what would be added
    -d, --depth N            search N levels deep (default: max_depth)
  prune [-n | --dry-run]     remove entries whose folder no longer exists

New projects and dependencies
  new <name>                 create a project from a template and register it
    -t, --template T         the template (default: asks, or empty)
    --in <folder>            where (default: new_project_dir, or 1st scan dir)
    --list                   show the templates
  deps [project] [list]      list dependencies; project is a name, number or
                             folder (default: the current folder)
  deps [project] add [--dev] <package>...   add, with the project's own tool
  deps [project] remove <package>...        remove

Dev environments (one container per group of projects; alias: envs)
  env [list]                 list environments and whether they run
  env create <name>          create one; adds [envs.<name>] to the config
    --dir <folder>           whose projects belong to it (repeatable)
    -t, --template T         its env template (default: bindle)
  env up | stop | restart <name>   start (mounting its projects), stop, restart
  env logs <name> [-f]       show its logs (-f follows them)
  env shell <name> [project] a shell in its container, in the project's folder
  env open <name> [link] [--project P]   open a link (default: the editor)
  env config <name>          edit its settings (.env)
  env path <name>            print the folder with its files
  env remove <name>          remove it: container, files, config (volumes stay)

Settings and info
  config                     edit config.toml in your editor
  config path                print where config.toml is
  tui                        open the TUI
  help                       show this help (also -h, --help)
  version                    print the version (also -V, --version)
```
`create`, `delete` and `new` ask for a missing argument.

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
rattle env remove work                     # container and files go; volumes stay
```
In the TUI, projects show their environment as `[work ●]` (● running, ○ stopped),
`o` opens the selected project in its environment, and the **Environments** tab
(`Tab`) manages them: start, stop, open, logs, folders, mounting, create, remove.
Removing keeps the environment's Docker volumes, so creating one with the same
name later gets its code and settings back.

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
browser VS Code box with Node, Python, Go, Docker and Claude Code. Its files come
from a fixed, reviewed bindle-le commit rather than its latest version, because
they start a privileged container; Rattle releases move that commit on purpose.

Every part of a template is configurable, so you can tweak it or bring your own
compose setup:

```toml
[env_templates.bindle]
compose = "~/templates/bindle/compose.yaml"   # a URL or a local file
env_file = "~/templates/bindle/.env.example"
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

## Contributing
Want to help? See [CONTRIBUTING.md](CONTRIBUTING.md) for setup, tests, the code
layout and how releases are made.

## Roadmap
- [x] Made Working MVP
- [x] Rewritten in Rust
- [x] Auto Scanning for predefined folder
- [x] Removing not only entries but whole directories (This one will be on config entry turnable)
- [x] Creating and manipulating packages
- [x] Dev environments: separate containers for groups of projects
- [x] One-line install script
- [ ] Dev environments on another machine (over SSH)

## License
MIT, see [LICENSE](LICENSE). The built-in bindle-le environment template is a
separate project with its own license (MIT); Rattle only downloads its files.
