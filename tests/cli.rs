mod support;

use std::process::Command;

use predicates::prelude::*;

use support::TestDir;

const ERROR_INCORRECT_ARGS: i32 = 2;

fn inf() -> assert_cmd::Command {
    assert_cmd::Command::cargo_bin("inf").expect("inf binary")
}

fn inf_in(dir: &TestDir) -> assert_cmd::Command {
    let mut cmd = inf();
    cmd.current_dir(dir.path());
    cmd
}

#[test]
fn no_arguments_prints_help_message() {
    inf()
        .assert()
        .code(ERROR_INCORRECT_ARGS)
        .stderr(predicate::str::contains("Usage:"));
}

#[test]
fn help_flag_prints_usage() {
    inf()
        .arg("-h")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage:"));
}

#[test]
fn version_flag_prints_version() {
    inf()
        .arg("-V")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("inf "));
}

#[test]
fn debug_flag_exits_after_printing_parsed_arguments() {
    let dir = TestDir::new();
    dir.init_default_files();

    inf_in(&dir)
        .args(["-d", "--", "gcc", "main.c"])
        .assert()
        .success()
        .stdout(predicate::str::contains("debug=1"));
}

#[test]
fn input_flag_adds_file_to_run_command() {
    let dir = TestDir::new();
    dir.init_default_files();

    inf_in(&dir)
        .args(["-d", "-r", "echo hi", "-i", "input.txt", "--", "gcc main.c"])
        .assert()
        .success()
        .stdout(predicate::str::contains("runCmd=echo hi <input.txt"));
}

#[test]
fn monitor_flag_adds_extra_files() {
    let dir = TestDir::new();
    dir.init_default_files();

    inf_in(&dir)
        .args(["-d", "-r", "echo hi", "-m", "extra.txt", "--", "gcc main.c"])
        .assert()
        .success()
        .stdout(predicate::str::contains("extra.txt"));
}

#[test]
fn noparse_flag_disables_compile_file_detection() {
    let dir = TestDir::new();
    dir.init_default_files();

    let output = inf_in(&dir)
        .args([
            "-d",
            "-n",
            "-r",
            "echo hi",
            "-m",
            "extra.txt",
            "--",
            "gcc main.c",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output = String::from_utf8(output).expect("utf8 stdout");
    let monitor_line = output
        .lines()
        .find(|line| line.contains("monitorFiles[*]"))
        .expect("monitor files line");

    assert!(!monitor_line.contains("main.c"));
}

#[test]
fn boolean_flags_are_parsed() {
    for (flag, rendered) in [
        ("-x", "refresh=1"),
        ("-p", "postpone=1"),
        ("-q", "quiet=1"),
        ("-w", "waitkey=1"),
        ("-z", "zen=1"),
        ("-v", "verbose=1"),
    ] {
        let dir = TestDir::new();
        dir.init_default_files();
        inf_in(&dir)
            .args(["-d", flag, "--", "gcc main.c"])
            .assert()
            .success()
            .stdout(predicate::str::contains(rendered));
    }
}

#[test]
fn monitor_flag_gets_comma_separated_args() {
    let dir = TestDir::new();
    dir.init_default_files();

    let output = inf_in(&dir)
        .args(["-d", "-m", "input.txt,extra.txt"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output = String::from_utf8(output).expect("utf8 stdout");
    let monitor_line = output
        .lines()
        .find(|line| line.contains("monitorFiles[*]"))
        .expect("monitor files line");

    assert!(monitor_line.contains("input.txt"));
    assert!(monitor_line.contains("extra.txt"));
}

#[test]
fn error_if_incorrect_arguments() {
    inf()
        .arg("-a")
        .assert()
        .code(ERROR_INCORRECT_ARGS)
        .stderr(predicate::str::contains("unexpected argument"));
}

#[test]
fn error_if_no_files_to_monitor() {
    inf()
        .args(["--", "echo hi"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("ERROR: no files to monitor"));
}

#[test]
fn error_if_some_explicitly_listed_files_are_missing() {
    let dir = TestDir::new();

    inf_in(&dir)
        .args(["-m", "noexist.txt", "--", "echo hi"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("ERROR: missing file"));
}

#[test]
fn error_if_input_without_run() {
    let dir = TestDir::new();
    dir.init_default_files();

    inf_in(&dir)
        .args(["-i", "input.txt", "--", "gcc main.c"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("ERROR: --input requires --run"));
}

#[test]
fn error_if_quiet_and_verbose() {
    let dir = TestDir::new();
    dir.init_default_files();

    inf_in(&dir)
        .args(["-q", "-v", "--", "gcc main.c"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "ERROR: --quiet and --verbose conflict",
        ));
}

#[test]
fn smoke_binary_can_be_spawned_without_assert_cmd() {
    let status = Command::new(support::inf_bin())
        .arg("-V")
        .status()
        .expect("spawn inf");
    assert!(status.success());
}
