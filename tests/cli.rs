//! End-to-end tests of the command-line interface, run against the real binary.

mod common;

use common::{canon, has_tool, Sandbox};

#[test]
fn create_show_and_delete_entries() {
    let sb = Sandbox::new("crud");
    let alpha = sb.dir("code/alpha");
    sb.dir("code/beta");

    let out = sb.run(&["create", alpha.to_str().unwrap()]).ok();
    assert!(out.contains("Added 'alpha'"), "{out}");
    assert!(sb
        .run(&["create", alpha.to_str().unwrap()])
        .err()
        .contains("already exists"));
    assert!(sb
        .run(&["create", "/no/such/folder"])
        .err()
        .contains("cannot access"));
    sb.run(&["add", "code/beta"]).ok(); // relative to the current folder

    let shown = sb.run(&["show"]).ok();
    assert!(
        shown.contains(" 0. alpha") && shown.contains(" 1. beta"),
        "{shown}"
    );

    sb.run(&["delete", "0"]).ok(); // by index from `show`
    assert_eq!(sb.registered(), vec!["beta"]);
    assert!(alpha.exists(), "deleting an entry must keep the folder");
    assert!(sb
        .run(&["delete", "ghost"])
        .err()
        .contains("no project named 'ghost'"));
}

#[test]
fn reads_registry_written_by_the_python_version() {
    let sb = Sandbox::new("legacy");
    std::fs::write(
        sb.registry(),
        "[old]\npath=/somewhere/old\n[other]\npath=/x/other",
    )
    .unwrap();
    let shown = sb.run(&["show"]).ok();
    assert!(
        shown.contains("old  ") && shown.contains("/x/other"),
        "{shown}"
    );
}

#[test]
fn help_and_unknown_commands() {
    let sb = Sandbox::new("help");
    // Without a terminal, plain `rattle` prints help instead of opening the TUI.
    assert!(sb.run(&[]).ok().contains("Usage: rattle"));
    let unknown = sb.run(&["bogus"]);
    assert!(!unknown.ok && unknown.stderr.contains("unknown command: bogus"));
}

#[test]
fn scan_searches_deep_but_stops_at_projects() {
    let sb = Sandbox::new("scan");
    for dir in [
        "code/flat/.git",
        "code/flat/crates/inner/.git", // inside a project: not registered
        "code/org/team/nested/.git",   // depth 3
        "code/a/b/c/too-deep/.git",    // depth 4
        "code/node_modules/dep/.git",  // ignored folder
        "code/.cache/hidden/.git",     // hidden folder
    ] {
        sb.dir(dir);
    }
    sb.file("code/rusty/Cargo.toml", "[package]\nname = \"rusty\"\n");
    // Relative scan dirs are under HOME, wherever rattle is started.
    sb.write_config("scan_dirs = [\"code\"]\n");

    let dry = sb.run(&["scan", "-n"]).ok();
    assert!(dry.contains("Would add 3 project(s)"), "{dry}");
    assert!(sb.registered().is_empty(), "a dry run saves nothing");

    sb.run(&["scan"]).ok();
    assert_eq!(sb.registered(), vec!["flat", "nested", "rusty"]);
    let again = sb.run(&["scan"]).ok();
    assert!(
        again.contains("Added 0 project(s), 3 already registered"),
        "{again}"
    );

    sb.run(&["scan", "--depth", "4"]).ok();
    assert!(sb.registered().contains(&"too-deep".to_string()));
    assert!(sb
        .run(&["scan", "--depth"])
        .err()
        .contains("--depth needs a number"));
    assert!(sb
        .run(&["scan", "--wat"])
        .err()
        .contains("unknown scan option"));
}

#[test]
fn scan_without_folders_explains_what_to_do() {
    let sb = Sandbox::new("scan-empty");
    let err = sb.run(&["scan"]).err();
    assert!(
        err.contains("nothing to scan") && err.contains("config.toml"),
        "{err}"
    );
    // The commented template is created on first use.
    assert!(sb.config_text().contains("scan_dirs = []"));
}

#[test]
fn prune_removes_only_missing_entries() {
    let sb = Sandbox::new("prune");
    sb.dir("code/keep/.git");
    sb.dir("code/gone/.git");
    sb.write_config("scan_dirs = [\"code\"]\n");
    sb.run(&["scan"]).ok();
    std::fs::remove_dir_all(sb.home.join("code/gone")).unwrap();

    // Without a terminal, scan only reports; it never removes on its own.
    let scan = sb.run(&["scan"]).ok();
    assert!(
        scan.contains("no longer exist: gone") && scan.contains("rattle prune"),
        "{scan}"
    );
    assert_eq!(sb.registered(), vec!["gone", "keep"]);

    assert!(sb.run(&["prune", "-n"]).ok().contains("Would remove 1"));
    assert_eq!(sb.registered().len(), 2);
    assert!(sb.run(&["prune"]).ok().contains("Removed 1"));
    assert_eq!(sb.registered(), vec!["keep"]);
}

#[test]
fn deleting_folders_needs_setting_terminal_and_a_safe_target() {
    let sb = Sandbox::new("delete-files");
    let proj = sb.dir("code/proj/.git");
    sb.dir("code/outer/inner/.git");
    sb.write_config("scan_dirs = [\"code\"]\n");
    sb.run(&["scan"]).ok();
    sb.run(&["create", "code/outer"]).ok();
    sb.run(&["create", "code"]).ok(); // the scan folder itself
    sb.run(&["create", "."]).ok(); // the home folder itself

    let err = sb.run(&["delete", "proj", "--files"]).err();
    assert!(err.contains("turned off"), "off by default: {err}");

    sb.write_config("scan_dirs = [\"code\"]\ndelete_files = \"permanent\"\n");
    let err = sb.run(&["delete", "proj", "--files"]).err();
    assert!(err.contains("interactive terminal"), "{err}");
    for (name, why) in [
        ("code", "scan folder"),
        ("outer", "other registered projects (inner)"),
        ("home", "refusing to delete"),
    ] {
        let err = sb.run(&["delete", name, "--files"]).err();
        assert!(err.contains(why), "{name}: {err}");
    }
    assert!(proj.exists() && sb.home.join("code/outer/inner").exists());
    assert!(sb
        .run(&["delete", "proj", "--everything"])
        .err()
        .contains("unknown delete option"));
}

#[test]
fn new_projects_from_templates() {
    let sb = Sandbox::new("new");
    sb.write_config(
        "scan_dirs = [\"code\"]\n[templates]\nnotes = \"mkdir {name} && echo '# {name}' > {name}/README.md\"\n\
         broken = \"definitely-not-a-real-tool {name}\"\n",
    );
    let list = sb.run(&["new", "--list"]).ok();
    assert!(
        list.contains("rust-lib") && list.contains("notes"),
        "{list}"
    );

    sb.run(&["new", "plain", "-t", "empty"]).ok();
    assert!(sb.home.join("code/plain/.git").is_dir());
    sb.run(&["new", "notes-app", "-t", "notes"]).ok();
    assert_eq!(
        std::fs::read_to_string(sb.home.join("code/notes-app/README.md")).unwrap(),
        "# notes-app\n"
    );
    assert_eq!(sb.registered(), vec!["plain", "notes-app"]);

    for bad in ["bad name", "../escape", ".hidden"] {
        assert!(
            sb.run(&["new", bad, "-t", "empty"])
                .err()
                .contains("invalid project name"),
            "{bad}"
        );
    }
    let dup = sb.run(&["new", "plain", "-t", "empty"]);
    assert!(
        !dup.stdout.contains("Creating"),
        "refused before announcing: {}",
        dup.stdout
    );
    assert!(dup.err().contains("already registered"));
    assert!(sb
        .run(&["new", "x", "-t", "nope"])
        .err()
        .contains("unknown template 'nope'"));
    sb.dir("code/taken");
    assert!(sb
        .run(&["new", "taken", "-t", "empty"])
        .err()
        .contains("already exists"));

    let err = sb.run(&["new", "fresh", "-t", "broken"]).err();
    assert!(err.contains("template 'broken' failed"), "{err}");
    assert!(
        !sb.home.join("code/fresh").exists(),
        "a failed template leaves no folder"
    );

    let other = sb.dir("elsewhere");
    sb.run(&[
        "new",
        "there",
        "-t",
        "empty",
        "--in",
        other.to_str().unwrap(),
    ])
    .ok();
    assert!(other.join("there/.git").is_dir());
}

#[test]
fn new_projects_with_installed_toolchains() {
    let sb = Sandbox::new("new-tools");
    sb.write_config("scan_dirs = [\"code\"]\n");
    // cargo is always there when these tests run; the others are optional.
    for (template, tool, created) in [
        ("rust", "cargo", "Cargo.toml"),
        ("rust-lib", "cargo", "src/lib.rs"),
        ("python", "uv", "pyproject.toml"),
        ("node", "npm", "package.json"),
        ("go", "go", "go.mod"),
    ] {
        if tool != "cargo" && !has_tool(tool) {
            eprintln!("skipping template '{template}': {tool} not installed");
            continue;
        }
        let name = format!("t-{template}");
        sb.run(&["new", &name, "-t", template]).ok();
        assert!(
            sb.home.join("code").join(&name).join(created).exists(),
            "{template} made {created}"
        );
    }
}

#[test]
fn deps_lists_dependencies_from_each_manifest() {
    let sb = Sandbox::new("deps-list");
    let cases = [
        (
            "rs",
            "Cargo.toml",
            "[package]\nname = \"rs\"\n[dependencies]\nserde = { version = \"1.0\", features = [\"derive\"] }\n\
             [dev-dependencies]\ntempfile = \"3\"\n",
            "cargo (Cargo.toml) · 2 dependencies",
            vec!["serde 1.0", "tempfile 3 [dev]"],
        ),
        (
            "js",
            "package.json",
            r#"{"dependencies":{"react":"^18.3.1"},"devDependencies":{"vite":"^5.4.0"}}"#,
            "npm (package.json)",
            vec!["react ^18.3.1", "vite ^5.4.0 [dev]"],
        ),
        (
            "py",
            "pyproject.toml",
            "[project]\nname = \"py\"\ndependencies = [\n  \"requests>=2.31\",\n]\n[dependency-groups]\ndev = [\"pytest\"]\n",
            "uv (pyproject.toml)",
            vec!["requests >=2.31", "pytest [dev]"],
        ),
        (
            "go",
            "go.mod",
            "module x\n\nrequire (\n\tgolang.org/x/sync v0.8.0\n)\n",
            "go (go.mod) · 1 dependency",
            vec!["golang.org/x/sync v0.8.0"],
        ),
    ];
    for (name, manifest, text, header, lines) in cases {
        sb.file(&format!("{name}/{manifest}"), text);
        let out = sb.run(&["deps", name, "list"]).ok();
        assert!(out.contains(header), "{name}: {out}");
        // Columns are padded to the longest name; compare with spacing collapsed.
        let squashed: Vec<String> = out
            .lines()
            .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect();
        for line in lines {
            assert!(
                squashed.iter().any(|l| l == line),
                "{name}: missing {line:?} in\n{out}"
            );
        }
    }
    // A pnpm lockfile switches the manager.
    sb.file("js/pnpm-lock.yaml", "");
    assert!(sb.run(&["deps", "js"]).ok().contains("pnpm (package.json)"));
    // Without a project, the current folder is used.
    let out = common::Run::from(
        sb.cmd(&["deps"])
            .current_dir(sb.home.join("rs"))
            .output()
            .unwrap(),
    )
    .ok();
    assert!(out.contains("cargo (Cargo.toml)"), "{out}");

    sb.dir("plain");
    assert!(sb
        .run(&["deps", "plain"])
        .err()
        .contains("no supported manifest"));
    assert!(sb
        .run(&["deps", "ghost"])
        .err()
        .contains("no project or folder named 'ghost'"));
    assert!(sb
        .run(&["deps", "rs", "add"])
        .err()
        .contains("usage: rattle deps"));
}

/// Talks to crates.io, so it only runs on request: `cargo test -- --ignored`.
#[test]
#[ignore = "needs network access to crates.io"]
fn deps_add_and_remove_with_cargo() {
    let sb = Sandbox::new("deps-cargo");
    sb.write_config("scan_dirs = [\"code\"]\n");
    sb.run(&["new", "app", "-t", "rust"]).ok();
    sb.run(&["deps", "app", "add", "itoa"]).ok();
    sb.run(&["deps", "app", "add", "--dev", "memchr"]).ok();
    let list = sb.run(&["deps", "app"]).ok();
    assert!(list.contains("itoa") && list.contains("[dev]"), "{list}");
    sb.run(&["deps", "app", "remove", "memchr"]).ok(); // picks --dev itself
    let list = sb.run(&["deps", "app"]).ok();
    assert!(list.contains("itoa") && !list.contains("memchr"), "{list}");
}

#[test]
fn config_command_uses_configured_editor_and_waits_for_gui_editors() {
    let sb = Sandbox::new("config");
    let log = sb.home.join("editor.log");
    // A fake GUI editor: named `zed`, so rattle should pass --wait.
    let zed = sb.script("bin/zed", &format!("echo \"$@\" >> '{}'", log.display()));
    let hx = sb.script("bin/hx", &format!("echo \"hx $@\" >> '{}'", log.display()));

    let path = sb.run(&["config", "path"]).ok();
    assert_eq!(path.trim(), canon(&sb.config_file()));
    assert!(sb
        .run(&["config", "bogus"])
        .err()
        .contains("unknown config option"));

    // $VISUAL is used when the config sets no editor.
    let run = sb.cmd(&["config"]).env("VISUAL", &hx).output().unwrap();
    assert!(run.status.success());
    // The config editor beats $VISUAL, and GUI editors get --wait for config editing.
    std::fs::write(
        sb.config_file(),
        format!("editor = \"{}\"\n", zed.display()),
    )
    .unwrap();
    let run = sb.cmd(&["config"]).env("VISUAL", &hx).output().unwrap();
    assert!(run.status.success());

    let log = std::fs::read_to_string(log).unwrap();
    let config = sb.config_file().display().to_string();
    assert_eq!(log, format!("hx {config}\n--wait {config}\n"));

    std::fs::write(sb.config_file(), "editor = \"/no/such/editor\"\n").unwrap();
    assert!(sb
        .run(&["config"])
        .err()
        .contains("could not run '/no/such/editor'"));
}
