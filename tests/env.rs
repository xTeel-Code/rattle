//! End-to-end tests of `rattle env` with a fake `docker` (records every call
//! and keeps a running/stopped state per environment) and a fake browser.

mod common;

use common::{canon, Sandbox, Tui};

/// A sandbox with work and side projects, a fake docker and browser, and the
/// bindle template pointed at local files (no network).
fn sandbox(name: &str, port_base: u16) -> Sandbox {
    let sb = Sandbox::new(name);
    sb.dir("code/work/api/.git");
    sb.dir("code/work/web/.git");
    sb.dir("code/side/game/.git");
    sb.file(
        "tpl/compose.yaml",
        "services:\n  bindle:\n    image: fake\n",
    );
    sb.file(
        "tpl/env.example",
        "COMPOSE_PROJECT_NAME=bindle-01\nPORT_CODE=18000       # BASE+0 editor\nPORT_FILES=18004\n#PASSWORD=change-me\n",
    );
    let log = sb.home.join("docker.log");
    sb.script(
        "bin/docker",
        &format!(
            "echo \"$(pwd) $*\" >> '{}'\nstate=\"$(pwd)/.fake-state\"\ncase \"$2\" in\n\
             ps) [ -f \"$state\" ] && printf '{{\"Service\":\"bindle\",\"State\":\"%s\"}}\\n' \"$(cat \"$state\")\" ;;\n\
             up) echo running > \"$state\" ;;\nstop) echo exited > \"$state\" ;;\nrestart) echo running > \"$state\" ;;\n\
             logs) echo 'fake log line' ;;\nexec) echo \"exec: $*\" ;;\ndown) rm -f \"$state\" ;;\nesac\nexit 0",
            log.display()
        ),
    );
    sb.script(
        "bin/browser",
        &format!("echo \"$1\" >> '{}'", sb.home.join("browser.log").display()),
    );
    sb.write_config(&format!(
        "scan_dirs = [\"code\"]\ndocker = \"{home}/bin/docker\"\nbrowser = \"{home}/bin/browser\"\n\
         [env_templates.bindle]\ncompose = \"{home}/tpl/compose.yaml\"\nenv_file = \"{home}/tpl/env.example\"\n\
         port_base = {port_base}\n",
        home = sb.home.display()
    ));
    sb.run(&["scan"]).ok();
    sb
}

fn read(sb: &Sandbox, rel: &str) -> String {
    std::fs::read_to_string(sb.home.join(rel)).unwrap_or_default()
}

#[test]
fn work_and_personal_get_separate_containers_and_mounts() {
    let sb = sandbox("env-split", 42000);
    assert!(sb.run(&["env"]).ok().contains("No environments yet"));

    let out = sb
        .run(&["env", "create", "work", "--dir", "code/work"])
        .ok();
    assert!(
        out.contains("Added [envs.work]") && out.contains("PORT_CODE=42000"),
        "{out}"
    );
    sb.run(&["env", "create", "personal", "--dir", "code/side"])
        .ok();
    assert!(sb.config_text().contains("[envs.personal]"));

    // Unique names and port ranges; the template's comments survive.
    let work_env = read(&sb, ".rattle/envs/work/.env");
    assert!(
        work_env.contains("COMPOSE_PROJECT_NAME=rattle-work"),
        "{work_env}"
    );
    assert!(
        work_env.contains("PORT_CODE=42000       # BASE+0 editor"),
        "{work_env}"
    );
    let personal_env = read(&sb, ".rattle/envs/personal/.env");
    assert!(personal_env.contains("COMPOSE_PROJECT_NAME=rattle-personal"));
    assert!(personal_env.contains("PORT_CODE=42100") && personal_env.contains("PORT_FILES=42104"));

    let list = sb.run(&["env", "list"]).ok();
    assert!(
        list.contains("work") && list.contains("2 project(s)") && list.contains("1 project(s)"),
        "{list}"
    );
    assert!(list.contains("stopped"), "{list}");

    let up = sb.run(&["env", "up", "work"]).ok();
    assert!(
        up.contains("http://127.0.0.1:42000/?folder=/workspace"),
        "{up}"
    );
    let log = read(&sb, "docker.log");
    let work_dir = canon(&sb.home.join(".rattle/envs/work"));
    assert!(log.contains(&format!("{work_dir} compose up -d")), "{log}");

    // The work container sees only work projects.
    let mounts = read(&sb, ".rattle/envs/work/compose.override.yaml");
    assert!(
        mounts.contains(&canon(&sb.home.join("code/work"))),
        "{mounts}"
    );
    assert!(mounts.contains("target: \"/workspace/work\""), "{mounts}");
    assert!(
        !mounts.contains("code/side"),
        "personal projects leak into work: {mounts}"
    );
    assert!(!sb
        .home
        .join(".rattle/envs/personal/compose.override.yaml")
        .exists());

    let list = sb.run(&["env"]).ok();
    let work_line = list.lines().find(|l| l.contains(" work ")).unwrap();
    assert!(
        work_line.contains("running") && work_line.contains("ports from 42000"),
        "{list}"
    );

    sb.run(&["env", "up", "personal"]).ok();
    let personal = read(&sb, ".rattle/envs/personal/compose.override.yaml");
    assert!(
        personal.contains("code/side") && !personal.contains("code/work"),
        "{personal}"
    );
}

#[test]
fn changed_mounts_warn_that_the_container_is_recreated() {
    let sb = sandbox("env-recreate", 42200);
    sb.run(&["env", "create", "work", "--dir", "code/work"])
        .ok();
    let first = sb.run(&["env", "up", "work"]).ok();
    assert!(!first.contains("recreated"), "{first}");
    let again = sb.run(&["env", "up", "work"]).ok();
    assert!(!again.contains("recreated"), "same mounts: {again}");

    // `create` writes folders under home as ~/…, keeping the config portable.
    assert!(
        sb.config_text().contains("dirs = [\"~/code/work\"]"),
        "{}",
        sb.config_text()
    );
    let config = sb.config_text().replace(
        "dirs = [\"~/code/work\"]",
        "dirs = [\"~/code/work\", \"~/code/side\"]",
    );
    sb.write_config(&config);
    let changed = sb.run(&["env", "up", "work"]).ok();
    assert!(
        changed.contains("the container will be recreated"),
        "{changed}"
    );
}

#[test]
fn open_shell_stop_logs_and_errors() {
    let sb = sandbox("env-actions", 42400);
    sb.run(&["env", "create", "work", "--dir", "code/work"])
        .ok();
    sb.run(&["env", "up", "work"]).ok();

    sb.run(&["env", "open", "work", "--project", "api"]).ok();
    sb.run(&["env", "open", "work", "files"]).ok();
    assert_eq!(
        read(&sb, "browser.log"),
        "http://127.0.0.1:42400/?folder=/workspace/work/api\nhttp://127.0.0.1:42404\n"
    );
    assert!(sb
        .run(&["env", "open", "work", "nope"])
        .err()
        .contains("no link 'nope' (available: editor, files)"));

    let shell = sb.run(&["env", "shell", "work", "api"]).ok();
    assert!(
        shell.contains("exec: compose exec -w /workspace/work/api -u vscode bindle bash"),
        "{shell}"
    );

    sb.run(&["env", "stop", "work"]).ok();
    assert!(sb.run(&["env"]).ok().contains("stopped"));
    assert!(sb
        .run(&["env", "logs", "work"])
        .ok()
        .contains("fake log line"));
    assert!(sb
        .run(&["env", "path", "work"])
        .ok()
        .trim()
        .ends_with(".rattle/envs/work"));

    assert!(sb
        .run(&["env", "up", "ghost"])
        .err()
        .contains("no environment named 'ghost' (configured: work)"));
    assert!(sb
        .run(&["env", "create", "Bad Name"])
        .err()
        .contains("invalid environment name"));
    assert!(sb
        .run(&["env", "create", "work"])
        .err()
        .contains("already exists"));
    assert!(sb
        .run(&["env", "create", "x", "-t", "nope"])
        .err()
        .contains("unknown env template 'nope'"));
    assert!(sb
        .run(&["env", "bogus"])
        .err()
        .contains("unknown env command"));

    // Defined in config but never created.
    let config = format!("{}\n[envs.later]\ndirs = []\n", sb.config_text());
    sb.write_config(&config);
    assert!(sb
        .run(&["env", "up", "later"])
        .err()
        .contains("isn't created yet; run: rattle env create later"));
    assert!(sb.run(&["env"]).ok().contains("not created"));
}

#[test]
fn mount_false_env_gets_no_mounts_and_docker_problems_are_reported() {
    let sb = sandbox("env-nomount", 42600);
    sb.run(&["env", "create", "box"]).ok();
    let config = sb
        .config_text()
        .replace("[envs.box]", "[envs.box]\nmount = false");
    sb.write_config(&config);
    sb.run(&["env", "up", "box"]).ok();
    assert!(!sb
        .home
        .join(".rattle/envs/box/compose.override.yaml")
        .exists());

    let missing = sb.config_text().replace(
        &format!("docker = \"{}/bin/docker\"", sb.home.display()),
        "docker = \"/no/such/docker\"",
    );
    sb.write_config(&missing);
    let list = sb.run(&["env"]).ok();
    assert!(
        list.contains("unknown") && list.contains("(/no/such/docker is not installed)"),
        "{list}"
    );
    assert!(sb
        .run(&["env", "up", "box"])
        .err()
        .contains("could not run /no/such/docker"));
}

/// A sandbox with "work" and "personal" environments already created.
fn with_two_envs(name: &str, port_base: u16) -> Sandbox {
    let sb = sandbox(name, port_base);
    sb.dir("code/loose/.git"); // in no environment
    sb.run(&["scan"]).ok();
    sb.run(&["env", "create", "work", "--dir", "code/work"])
        .ok();
    sb.run(&["env", "create", "personal", "--dir", "code/side"])
        .ok();
    sb
}

#[test]
fn tui_tags_projects_and_starts_and_stops_environments() {
    let sb = with_two_envs("env-tui", 43000);
    let Some(tui) = Tui::start(&sb, "env-tui", &[]) else {
        return;
    };
    tui.wait_for("[work ○]");
    let screen = tui.wait_for("[personal ○]");
    let loose = screen.lines().find(|l| l.contains("loose")).unwrap();
    assert!(
        !loose.contains('['),
        "a project in no environment has no tag: {loose}"
    );

    tui.keys(&["Tab"]);
    tui.wait_for(" Environments (2) ");
    tui.wait_for("○ stopped");
    // The detail pane of the selected one.
    tui.wait_for("folders   ~/code/work");
    tui.wait_for("projects  api, web");
    tui.wait_for("1 editor  http://127.0.0.1:43000/?folder=/workspace");
    tui.keys(&["u"]);
    tui.wait_for("starting 'work': done");
    tui.wait_for("● running");
    let log = read(&sb, "docker.log");
    assert!(
        log.contains(&format!(
            "{} compose up -d",
            canon(&sb.home.join(".rattle/envs/work"))
        )),
        "{log}"
    );

    tui.keys(&["s"]);
    tui.wait_for("stopping 'work': done");
    tui.keys(&["j", "u"]); // the second one: personal
    tui.wait_for("starting 'personal': done");
    tui.keys(&["Escape"]);
    tui.wait_for("[personal ●]");
    tui.wait_for("[work ○]");
}

#[test]
fn tui_o_starts_the_environment_then_opens_the_project() {
    let sb = with_two_envs("env-tui-open", 43200);
    let Some(tui) = Tui::start(&sb, "env-tui-open", &[]) else {
        return;
    };
    tui.wait_for("[work ○]");
    tui.keys(&["/"]);
    tui.type_text("api");
    tui.keys(&["Enter", "o"]);
    // Starts it first ("starting 'work'"), then opens the link at the project.
    tui.wait_for("opened http://127.0.0.1:43200/?folder=/workspace/work/api");
    // The browser is launched without waiting for it; give it a moment.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while read(&sb, "browser.log").is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert_eq!(
        read(&sb, "browser.log"),
        "http://127.0.0.1:43200/?folder=/workspace/work/api\n"
    );
    tui.wait_for("[work ●]");

    // Already running: opens at once, without starting it again.
    tui.keys(&["o"]);
    tui.wait_for("opened");
    let ups = read(&sb, "docker.log").matches("compose up -d").count();
    assert_eq!(ups, 1, "{}", read(&sb, "docker.log"));

    tui.keys(&["Escape", "/"]);
    tui.type_text("loose");
    tui.keys(&["Enter", "o"]);
    tui.wait_for("'loose' isn't in an environment");
}

#[test]
fn tui_creates_an_environment_and_shows_logs_and_links() {
    let sb = sandbox("env-tui-new", 43400);
    let Some(tui) = Tui::start(&sb, "env-tui-new", &[]) else {
        return;
    };
    tui.wait_for("Projects (");
    tui.keys(&["v"]);
    tui.wait_for("No environments yet.");
    tui.wait_for("Press n to create one.");
    tui.keys(&["n"]);
    tui.type_text("side");
    tui.keys(&["Enter"]);
    tui.wait_for("Folders for 'side'");
    tui.type_text("~/code/side");
    tui.keys(&["Enter"]);
    tui.wait_for("created 'side'");
    assert!(
        sb.config_text()
            .contains("[envs.side]\ntemplate = \"bindle\"\ndirs = [\"~/code/side\"]"),
        "{}",
        sb.config_text()
    );
    assert!(sb.home.join(".rattle/envs/side/compose.yaml").is_file());
    tui.wait_for("1 project(s)");

    tui.keys(&["l"]);
    tui.wait_for("logs: side");
    tui.wait_for("fake log line");
    tui.keys(&["x"]); // back to the list
    tui.wait_for("2 files  http://127.0.0.1:43404");
    tui.keys(&["2"]); // opens link 2 (starts the environment first)
    tui.wait_for("opened http://127.0.0.1:43404");
    tui.keys(&["n"]);
    tui.type_text("Bad Name");
    tui.keys(&["Enter", "Enter"]);
    tui.wait_for("invalid environment name 'Bad Name'");
}

#[test]
fn tui_envs_tab_changes_folders_and_mounting() {
    let sb = with_two_envs("env-tui-edit", 43600);
    let Some(tui) = Tui::start(&sb, "env-tui-edit", &[]) else {
        return;
    };
    tui.wait_for("[work ○]");
    tui.keys(&["Tab"]);
    tui.wait_for("folders   ~/code/work");

    // d: the current folders are prefilled; replace them.
    tui.keys(&["d"]);
    tui.wait_for("Folders for 'work' (comma-separated): ~/code/work");
    for _ in 0.."~/code/work".len() {
        tui.keys(&["BSpace"]);
    }
    tui.type_text("~/code/work, ~/code/side");
    tui.keys(&["Enter"]);
    tui.wait_for("folders saved; they apply when 'work' starts");
    tui.wait_for("folders   ~/code/work, ~/code/side");
    let config = sb.config_text();
    assert!(
        config.contains(
            "[envs.work]\ntemplate = \"bindle\"\ndirs = [\"~/code/work\", \"~/code/side\"]\n"
        ),
        "{config}"
    );
    assert!(
        config.contains("[envs.personal]"),
        "other environments are untouched: {config}"
    );
    tui.wait_for("projects  api, game, web");

    // m: toggle mounting, written to its section.
    tui.keys(&["m"]);
    tui.wait_for("projects won't be mounted");
    tui.wait_for("no: code lives only inside");
    assert!(
        sb.config_text().contains("[envs.work]\nmount = false\n"),
        "{}",
        sb.config_text()
    );
    tui.keys(&["m"]);
    tui.wait_for("yes: its projects are mounted");

    // Tab and Esc go back to Projects; the tags follow the new folders.
    tui.keys(&["Escape"]);
    tui.wait_for("Projects (4)");
    let screen = tui.wait_for("game");
    let game = screen.lines().find(|l| l.contains("game")).unwrap();
    assert!(game.contains("[work"), "game now belongs to work: {game}");
}

#[test]
fn tui_envs_tab_removes_an_environment_after_typing_its_name() {
    let sb = with_two_envs("env-tui-remove", 43800);
    sb.run(&["env", "up", "work"]).ok();
    let work_dir = sb.home.join(".rattle/envs/work");
    let work_canon = canon(&work_dir); // before it is removed
    let Some(tui) = Tui::start(&sb, "env-tui-remove", &[]) else {
        return;
    };
    tui.wait_for("[work ●]");
    tui.keys(&["Tab"]);
    tui.wait_for("● running");

    // A wrong name cancels.
    tui.keys(&["D"]);
    tui.wait_for("Remove 'work' (container and files; volumes stay)? Type 'work':");
    tui.type_text("wrk");
    tui.keys(&["Enter"]);
    tui.wait_for("cancelled; nothing was removed");
    assert!(work_dir.is_dir() && sb.config_text().contains("[envs.work]"));

    tui.keys(&["D"]);
    tui.type_text("work");
    tui.keys(&["Enter"]);
    tui.wait_for("removed 'work'; its volumes are kept");
    tui.wait_for(" Environments (1) ");
    let log = read(&sb, "docker.log");
    assert!(log.contains(&format!("{work_canon} compose down")), "{log}");
    assert!(
        !log.contains("down -v"),
        "volumes must never be removed: {log}"
    );
    assert!(!work_dir.exists(), "its files are removed");
    let config = sb.config_text();
    assert!(
        !config.contains("[envs.work]") && config.contains("[envs.personal]"),
        "{config}"
    );

    // Back in Projects, work's projects no longer have a tag.
    tui.keys(&["Tab"]);
    let screen = tui.wait_for("Projects (");
    let api = screen.lines().find(|l| l.contains(" api ")).unwrap();
    assert!(!api.contains("[work"), "{api}");
}

#[test]
fn cli_env_remove_needs_a_terminal() {
    let sb = with_two_envs("env-remove-cli", 44000);
    let err = sb.run(&["env", "remove", "work"]).err();
    assert!(err.contains("needs an interactive terminal"), "{err}");
    assert!(sb.config_text().contains("[envs.work]"));
}
