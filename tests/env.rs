//! End-to-end tests of `rattle env` with a fake `docker` (records every call
//! and keeps a running/stopped state per environment) and a fake browser.

mod common;

use common::{canon, Sandbox};

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
             logs) echo 'fake log line' ;;\nexec) echo \"exec: $*\" ;;\nesac\nexit 0",
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
