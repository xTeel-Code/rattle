# Contributing to Rattle

Thanks for helping out! This covers working on Rattle itself; for using it, see
the [README](README.md).

## Setup
You need [Rust](https://rustup.rs) (stable). For the TUI tests, also `tmux`.
```sh
git clone https://github.com/xTeel-Code/rattle && cd rattle
cargo run               # the TUI, using your real ~/.rattle
cargo run -- help       # any command
```
To try changes without touching your own projects, point Rattle at a scratch
registry: `RATTLE_FILE=/tmp/rattle-dev/projects.toml cargo run`.

## Checks
CI runs these on every pull request (Linux and macOS); run them before pushing:
```sh
cargo fmt                              # format
cargo clippy --all-targets             # lint (CI treats warnings as errors)
cargo test                             # unit + end-to-end tests
cargo test -- --ignored                # also tests that need network access
sh tests/install_test.sh               # install.sh against a local fake release
```
CI additionally runs `shellcheck` on the shell scripts and builds the static
Linux release target.

## Tests
The end-to-end tests run the real binary in a throwaway home folder, so they
never touch your `~/.rattle`:

| File | Covers |
| --- | --- |
| `tests/cli.rs` | the commands |
| `tests/env.rs` | dev environments, with a fake `docker` and browser |
| `tests/tui.rs` | the TUI, driven inside tmux (skipped when tmux isn't installed) |
| `tests/install_test.sh` | `install.sh`, against a release served from 127.0.0.1 |

TUI tests wait for text on screen instead of sleeping, so they stay reliable on
slow machines; keep it that way when adding one.

## Code layout
| | |
| --- | --- |
| `src/main.rs` | command-line commands |
| `src/tui/` | the TUI: `mod.rs` keys and actions, `draw.rs` rendering, one file per popup, `job.rs` background commands |
| `src/store.rs` | the project registry (`projects.toml`) |
| `src/config.rs` | `config.toml`: parsing, the commented template, in-place edits |
| `src/settings.rs` | the TUI settings form (`,`) |
| `src/ops.rs` | scanning, folder deletion checks, editor launching |
| `src/templates.rs` | `rattle new` project templates |
| `src/deps.rs` | dependency listing and package-manager commands |
| `src/envs.rs` | dev environments: env templates, mounts, ports, `docker compose` |

## Pull requests
- Branch from `main` and keep each PR to one change.
- Add or update tests for behaviour changes, and the README for user-facing ones.
- CI must be green before merging.

## Releasing
For maintainers. Releases are built and published by GitHub Actions from a tag.

Two things are pinned to exact commits on purpose, and are bumped only after
reading what changed:
- The actions in `.github/workflows/` (the tag stays in a comment). The release
  workflow can publish binaries everyone installs, so a moved tag upstream must
  not change what runs. `dtolnay/rust-toolchain` then needs `toolchain: stable`.
- The built-in bindle template (`BINDLE_RAW` in `src/envs.rs`), which starts a
  privileged container.

1. Bump `version` in `Cargo.toml` and run `cargo build`, which updates
   `Cargo.lock`. Release builds use `--locked`, so a stale lockfile fails them.
   Merge that to `main` through a PR.
2. Tag the merged commit and push the tag (it must start with `v`):
   ```sh
   git checkout main && git pull
   git tag -a v0.4.0 -m "Rattle v0.4.0"
   git push origin v0.4.0
   ```
3. The release workflow builds binaries for macOS (arm64, x86_64) and Linux
   (x86_64, arm64; static musl), uploads them with SHA-256 checksums to a draft
   release with generated notes, and publishes it once all builds succeed.
   Follow it with `gh run watch` or the Actions tab.
4. Check it: `gh release view v0.4.0`, then run the install command from the
   README; it should say "checksum ok" rather than building from source.

If a build fails, the release stays a draft, so nobody gets a broken one. Fix
the problem, then remove the draft and the tag and tag again:
```sh
gh release delete v0.4.0 --yes
git push --delete origin v0.4.0 && git tag -d v0.4.0
```
