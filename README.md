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
```
Omitted arguments are asked for interactively.

Projects are stored in `~/.rattle/projects.toml` (override with `RATTLE_FILE`).
Files written by the old Python version are read without changes.

## Roadmap
- [x] Made Working MVP
- [x] Rewritten in Rust
- [ ] Auto Scanning for predefined folder
- [ ] Removing not only entries but whole directories (This one will be on config entry turnable)
- [ ] Creating and manipulating packages
