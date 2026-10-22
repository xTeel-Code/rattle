//! End-to-end tests of the TUI: the real binary runs inside tmux, keys are
//! sent to it and the screen is read back. Skipped when tmux is missing
//! (CI sets RATTLE_REQUIRE_TMUX=1 so they always run there).

mod common;

use std::time::Duration;

use common::{canon, Sandbox, Tui};

/// A sandbox with two scannable projects under ~/code.
fn with_projects(name: &str) -> Sandbox {
    let sb = Sandbox::new(name);
    sb.dir("code/alpha/.git");
    sb.dir("code/team/beta/.git");
    sb.write_config("scan_dirs = [\"code\"]\n");
    sb
}

#[test]
fn empty_state_then_scan_and_filter() {
    let sb = Sandbox::new("tui-empty");
    let Some(tui) = Tui::start(&sb, "empty", &[]) else {
        return;
    };
    tui.wait_for("No projects yet. Press 'a'");

    sb.dir("code/alpha/.git");
    sb.write_config("scan_dirs = [\"code\"]\n");
    tui.keys(&["s"]);
    tui.wait_for("scan: added 1, 0 already registered");
    tui.wait_for("alpha");

    tui.keys(&["/"]);
    tui.type_text("zzz");
    tui.wait_for("No project matches the filter.");
    tui.keys(&["Escape"]);
    tui.wait_for("Projects (1)");
}

#[test]
fn auto_scan_on_open_and_enter_prints_the_path() {
    let sb = with_projects("tui-enter");
    let Some(tui) = Tui::start(&sb, "enter", &[]) else {
        return;
    };
    tui.wait_for("auto-scan: added 2");
    tui.keys(&["/"]);
    tui.type_text("beta");
    tui.keys(&["Enter"]);
    tui.wait_for("filter: beta");
    // Past the startup guard, Enter picks the project and quits.
    std::thread::sleep(Duration::from_millis(700));
    tui.keys(&["Enter"]);
    let out = tui.wait_exit();
    assert_eq!(
        out,
        format!("{}\nEXIT=0\n", canon(&sb.home.join("code/team/beta")))
    );
}

/// Regression test: the Enter that launches rattle (key repeat, or pressed
/// twice) used to select the first project and quit immediately.
#[test]
fn enter_right_after_start_is_ignored() {
    let sb = with_projects("tui-guard");
    let Some(tui) = Tui::start(&sb, "guard", &[]) else {
        return;
    };
    tui.keys(&["Enter"]);
    tui.keys(&["Enter"]);
    tui.wait_for("Projects (2)");
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !tui.has_exited(),
        "early Enter must not quit:\n{}",
        tui.screen()
    );
}

/// The guard only covers leftovers from launching: once another key was
/// pressed, Enter works at once, however soon after start.
#[test]
fn enter_after_another_key_works_immediately() {
    let sb = with_projects("tui-guard-key");
    let Some(tui) = Tui::start(&sb, "guard-key", &[]) else {
        return;
    };
    tui.wait_for("Projects (2)");
    tui.keys(&["j", "Enter"]);
    let beta = canon(&sb.home.join("code/team/beta"));
    assert_eq!(tui.wait_exit(), format!("{beta}\nEXIT=0\n"));
}

/// Regression test: the TUI used to save its own stale copy of the
/// registry, erasing projects added in another terminal meanwhile.
#[test]
fn changes_from_another_terminal_survive_tui_edits() {
    let sb = with_projects("tui-concurrent");
    let Some(tui) = Tui::start(&sb, "concurrent", &[]) else {
        return;
    };
    tui.wait_for("auto-scan: added 2");
    let outside = sb.dir("elsewhere/outside");
    sb.run(&["create", outside.to_str().unwrap()]).ok();

    tui.keys(&["d"]); // removes the selected project: alpha
    tui.wait_for("Remove 'alpha' from Rattle?");
    tui.keys(&["y"]);
    tui.wait_for("removed 'alpha' (files kept)");
    let mut registered = sb.registered();
    registered.sort();
    assert_eq!(registered, vec!["beta", "outside"]);
    // And the TUI now shows the project added from outside.
    tui.wait_for("outside");
}

#[test]
fn add_and_remove_an_entry() {
    let sb = Sandbox::new("tui-add");
    let extra = sb.dir("code/extra");
    let Some(tui) = Tui::start(&sb, "add", &[]) else {
        return;
    };
    tui.wait_for("No projects yet");

    tui.keys(&["a"]);
    tui.wait_for("Add project folder");
    tui.type_text(extra.to_str().unwrap());
    tui.keys(&["Enter"]);
    tui.wait_for("added 'extra'");
    assert_eq!(sb.registered(), vec!["extra"]);

    tui.keys(&["d"]);
    tui.wait_for("Remove 'extra' from Rattle?");
    tui.keys(&["y"]);
    tui.wait_for("removed 'extra' (files kept)");
    assert!(sb.registered().is_empty());
    assert!(extra.exists());
}

#[test]
fn x_removes_entries_whose_folder_is_gone() {
    let sb = with_projects("tui-prune");
    sb.run(&["scan"]).ok();
    std::fs::remove_dir_all(sb.home.join("code/alpha")).unwrap();
    let Some(tui) = Tui::start(&sb, "prune", &[]) else {
        return;
    };
    tui.wait_for("1 missing, press X to remove");
    tui.wait_for("missing");

    tui.keys(&["X"]);
    tui.wait_for("Remove 1 missing project(s) from Rattle?");
    tui.keys(&["y"]);
    tui.wait_for("removed 1 missing project(s)");
    assert_eq!(sb.registered(), vec!["beta"]);
}

#[test]
fn deleting_a_folder_needs_the_setting_and_the_typed_name() {
    let sb = with_projects("tui-delete");
    let Some(tui) = Tui::start(&sb, "delete", &[]) else {
        return;
    };
    tui.wait_for("auto-scan: added 2");
    tui.keys(&["D"]);
    tui.wait_for("deleting folders is off");

    sb.write_config("scan_dirs = [\"code\"]\ndelete_files = \"permanent\"\n");
    let alpha = sb.home.join("code/alpha");
    tui.keys(&["D"]);
    tui.wait_for("Permanently delete this folder");
    tui.type_text("alphX");
    tui.keys(&["Enter"]);
    tui.wait_for("cancelled; nothing was deleted");
    assert!(alpha.exists());

    tui.keys(&["D"]);
    tui.wait_for("Type 'alpha'");
    tui.type_text("alpha");
    tui.keys(&["Enter"]);
    tui.wait_for("removed 'alpha'; folder deleted");
    assert!(!alpha.exists());
    assert_eq!(sb.registered(), vec!["beta"]);
}

#[test]
fn n_creates_a_project_from_a_template() {
    let sb = Sandbox::new("tui-new");
    sb.write_config("scan_dirs = [\"code\"]\n");
    let Some(tui) = Tui::start(&sb, "new", &[]) else {
        return;
    };
    tui.wait_for("No projects yet");

    tui.keys(&["n"]);
    tui.wait_for("New project in");
    tui.type_text("bad name");
    tui.keys(&["Enter"]);
    tui.wait_for("invalid project name 'bad name'");

    for _ in 0.."bad name".len() {
        tui.keys(&["BSpace"]);
    }
    tui.type_text("made-here");
    tui.keys(&["Enter"]);
    tui.wait_for("Template for 'made-here'");
    tui.keys(&["Enter"]); // the first template: empty
    tui.wait_for("created 'made-here'");
    assert!(sb.home.join("code/made-here/.git").is_dir());
    assert_eq!(sb.registered(), vec!["made-here"]);
}

/// Templates run in the background: the TUI shows a spinner, and Esc stops
/// the template together with everything it started.
#[test]
fn slow_template_runs_in_background_and_esc_cancels_it() {
    let sb = Sandbox::new("tui-cancel");
    let marker = sb.home.join("child-survived");
    // A background child that would leave a marker if only `sh` were killed.
    sb.write_config(&format!(
        "scan_dirs = [\"code\"]\n[templates]\nempty = \"(sleep 1.5; touch '{}') & sleep 30; mkdir {{name}}\"\n",
        marker.display()
    ));
    let Some(tui) = Tui::start(&sb, "cancel", &[]) else {
        return;
    };
    tui.wait_for("No projects yet");
    tui.keys(&["n"]);
    tui.wait_for("New project in");
    tui.type_text("slow");
    tui.keys(&["Enter"]);
    tui.wait_for("Template for 'slow'");
    tui.keys(&["Enter"]);
    tui.wait_for("creating 'slow' with template 'empty'");
    tui.wait_for("Esc cancels");

    tui.keys(&["q"]); // ignored while the job runs
    std::thread::sleep(Duration::from_millis(200));
    assert!(!tui.has_exited(), "q must not quit during a job");
    tui.keys(&["Escape"]);
    tui.wait_for("cancelled creating 'slow'");

    std::thread::sleep(Duration::from_millis(2000));
    assert!(
        !marker.exists(),
        "the template's child process kept running"
    );
    assert!(!sb.home.join("code/slow").exists());
    assert!(sb.registered().is_empty());
}

/// Regression test: a process left behind by a template (here one that also
/// ignores SIGTERM) keeps the output pipes open; the TUI must not freeze.
#[test]
fn template_leaving_a_process_behind_does_not_freeze_the_tui() {
    let sb = Sandbox::new("tui-linger");
    sb.write_config(
        "scan_dirs = [\"code\"]\n[templates]\nempty = \"(trap '' TERM; sleep 8) & mkdir {name}\"\n",
    );
    let Some(tui) = Tui::start(&sb, "linger", &[]) else {
        return;
    };
    tui.wait_for("No projects yet");
    tui.keys(&["n"]);
    tui.type_text("lingers");
    tui.keys(&["Enter"]);
    tui.wait_for("Template for 'lingers'");
    let started = std::time::Instant::now();
    tui.keys(&["Enter"]);
    tui.wait_for("created 'lingers'");
    // The leftover process lives 8s; finishing well before that proves the TUI didn't wait for it.
    assert!(
        started.elapsed() < Duration::from_secs(6),
        "took {:?}",
        started.elapsed()
    );
    assert_eq!(sb.registered(), vec!["lingers"]);
}

/// Dependency commands run in the background too; a fake `cargo` on PATH
/// keeps this offline.
#[test]
fn p_adds_a_dependency_in_the_background() {
    let sb = Sandbox::new("tui-deps-add");
    sb.file(
        "code/app/Cargo.toml",
        "[package]\nname = \"app\"\n[dependencies]\nitoa = \"1\"\n",
    );
    sb.write_config("scan_dirs = [\"code\"]\n");
    let bin = sb.script(
        "bin/cargo",
        "sleep 1\necho \"      Adding $2 v9.9.9 to dependencies\" >&2\nprintf '%s = \"9.9.9\"\\n' \"$2\" >> Cargo.toml",
    );
    let path = format!(
        "{}:{}",
        bin.parent().unwrap().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let Some(tui) = Tui::start(&sb, "deps-add", &[("PATH", &path)]) else {
        return;
    };
    tui.wait_for("auto-scan: added 1");
    tui.keys(&["p"]);
    tui.wait_for("Dependencies · app");
    tui.keys(&["a"]);
    tui.type_text("newdep");
    tui.keys(&["Enter"]);
    tui.wait_for("cargo add newdep · ");
    tui.wait_for("cargo add newdep: Adding newdep v9.9.9");
    tui.wait_for("9.9.9");
}

#[test]
fn p_lists_dependencies() {
    let sb = Sandbox::new("tui-deps");
    sb.file(
        "code/app/Cargo.toml",
        "[package]\nname = \"app\"\n[dependencies]\nitoa = \"1\"\n[dev-dependencies]\nmemchr = \"2\"\n",
    );
    sb.write_config("scan_dirs = [\"code\"]\n");
    let Some(tui) = Tui::start(&sb, "deps", &[]) else {
        return;
    };
    tui.wait_for("auto-scan: added 1");

    tui.keys(&["p"]);
    tui.wait_for("Dependencies · app · cargo (Cargo.toml)");
    // Wait for each line: a capture can land while a frame is half painted.
    for text in ["itoa", "memchr", "dev"] {
        tui.wait_for(text);
    }
    tui.keys(&["a"]);
    tui.wait_for(" add: ");
    tui.keys(&["Escape"]);
    tui.keys(&["Escape"]);
    tui.wait_for("Projects (1)");
}

#[test]
fn settings_form_saves_only_changes_and_keeps_comments() {
    let sb = with_projects("tui-settings");
    sb.write_config("# my roots\nscan_dirs = [\"code\"]\n# editor = \"hx\"\n");
    let Some(tui) = Tui::start(&sb, "settings", &[]) else {
        return;
    };
    tui.wait_for("auto-scan: added 2");

    tui.keys(&[","]);
    tui.wait_for("Settings · config.toml");
    for _ in 0..4 {
        tui.keys(&["j"]); // down to Editor
    }
    tui.keys(&["Enter"]);
    tui.type_text("zed");
    tui.keys(&["Enter"]);
    tui.wait_for("*Editor");
    tui.keys(&["Escape"]);
    tui.wait_for("unsaved changes");
    tui.keys(&["s"]);
    tui.wait_for("settings saved (1 changed)");
    assert_eq!(
        sb.config_text(),
        "# my roots\nscan_dirs = [\"code\"]\n# editor = \"hx\"\neditor = \"zed\"\n"
    );
}

#[test]
fn c_edits_the_config_then_reloads_and_rescans() {
    let sb = Sandbox::new("tui-config");
    sb.dir("code/alpha/.git");
    // A fake editor that sets scan_dirs in the file it is given.
    let editor = sb.script(
        "bin/fake-editor",
        "printf 'scan_dirs = [\"code\"]\\n' >> \"$1\"",
    );
    sb.write_config(&format!("editor = \"{}\"\n", editor.display()));
    let Some(tui) = Tui::start(&sb, "config", &[]) else {
        return;
    };
    tui.wait_for("No projects yet");
    tui.keys(&["c"]);
    tui.wait_for("config reloaded · scan: added 1");
    tui.wait_for("alpha");
}

/// Regression test: returning from the editor used to crash with "cursor
/// position could not be read" when stdout was captured, as in `cd "$(rattle)"`
/// (the tests always capture stdout).
#[test]
fn e_opens_the_editor_in_the_project_and_comes_back() {
    let sb = with_projects("tui-editor");
    let log = sb.home.join("editor.log");
    let editor = sb.script("bin/fake-editor", &format!("pwd > '{}'", log.display()));
    sb.write_config(&format!(
        "scan_dirs = [\"code\"]\neditor = \"{}\"\n",
        editor.display()
    ));
    let Some(tui) = Tui::start(&sb, "editor", &[]) else {
        return;
    };
    tui.wait_for("auto-scan: added 2");

    tui.keys(&["e"]);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !log.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "editor never ran:\n{}",
            tui.screen()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let alpha = canon(&sb.home.join("code/alpha"));
    assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), alpha);

    tui.wait_for("Projects (2)");
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !tui.has_exited(),
        "rattle quit after the editor:\n{}",
        tui.screen()
    );
    tui.keys(&["Enter"]);
    assert_eq!(tui.wait_exit(), format!("{alpha}\nEXIT=0\n"));
}

/// In a short terminal the key list scrolls; in a narrow one the footer keeps
/// the essential hints instead of being cut off.
#[test]
fn help_scrolls_and_footer_fits_in_a_small_terminal() {
    let sb = Sandbox::new("tui-small");
    let Some(tui) = Tui::start_sized(&sb, "small", &[], (50, 12)) else {
        return;
    };
    tui.wait_for("No projects yet");
    // Wait for the footer itself: a capture can land on a half-painted frame.
    let screen = tui.wait_for("q quit");
    let footer = screen.lines().last().unwrap_or_default();
    assert!(
        footer.contains("? keys") && footer.contains("q quit") && !footer.contains(", settings"),
        "footer: {footer:?}"
    );

    tui.keys(&["?"]);
    tui.wait_for(" ↓ j/k ");
    for _ in 0..30 {
        tui.keys(&["j"]); // past the end: clamped
    }
    tui.wait_for("back to Proj"); // the last key (cut off in 50 columns)
    tui.wait_for("↑  j/k");
    tui.keys(&["k"]);
    tui.wait_for("↑↓ j/k"); // one step back up works at once
    tui.keys(&["x"]); // any other key closes
    tui.wait_for("No projects yet");
    assert!(!tui.screen().contains(" Keys "));
}

#[test]
fn help_lists_all_keys() {
    let sb = Sandbox::new("tui-help");
    let Some(tui) = Tui::start(&sb, "help", &[]) else {
        return;
    };
    tui.wait_for("No projects yet");
    tui.keys(&["?"]);
    tui.wait_for(" Keys ");
    // Wait for each line: a capture can land while a frame is half painted.
    for key in [
        "new project from a template",
        "dependencies",
        "settings form",
        "remove entries whose folder",
    ] {
        tui.wait_for(key);
    }
    tui.keys(&["x"]);
    tui.keys(&["q"]);
    tui.wait_exit();
}
