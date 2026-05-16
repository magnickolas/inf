mod support;

use std::time::Duration;

use support::{
    RunningInf, TestDir, assert_line_count_stays, exact_line_count, trigger_until_line_count,
};

fn run_until(cwd: &std::path::Path, args: &[&str], line: &str) -> String {
    let mut inf = RunningInf::spawn(cwd, args);
    let output = inf
        .wait_for_line_count_at_least(line, 1, Duration::from_secs(5))
        .unwrap_or_else(|err| panic!("timed out waiting for {line:?}: {err}"));
    inf.stop();
    output
}

#[test]
fn run_command_output_is_printed() {
    let dir = TestDir::new();
    dir.init_default_files();

    let output = run_until(
        dir.path(),
        &["-m", "input.txt", "-r", "echo run", "--", "echo compile"],
        "[execution succeeded]",
    );

    assert_eq!(
        output,
        "[compilation: echo compile]\necho run\nrun\n[execution succeeded]\n"
    );
}

#[test]
fn run_command_output_is_printed_in_zen_mode() {
    let dir = TestDir::new();
    dir.init_default_files();

    let output = run_until(
        dir.path(),
        &[
            "-z",
            "-m",
            "input.txt",
            "-r",
            "echo run",
            "--",
            "echo compile",
        ],
        "run",
    );

    assert_eq!(output, "run\n");
}

#[test]
fn compile_and_run_command_output_is_printed_in_verbose_mode() {
    let dir = TestDir::new();
    dir.init_default_files();

    let output = run_until(
        dir.path(),
        &[
            "-v",
            "-m",
            "input.txt",
            "-r",
            "echo run",
            "--",
            "echo compile",
        ],
        "[execution succeeded]",
    );

    assert_eq!(
        output,
        "[compilation: echo compile]\ncompile\necho run\nrun\n[execution succeeded]\n"
    );
}

#[test]
fn compile_command_error_output_is_printed() {
    let dir = TestDir::new();
    dir.init_default_files();

    let output = run_until(
        dir.path(),
        &[
            "-m",
            "input.txt",
            "-r",
            "echo run",
            "--",
            "echo compile >&2",
        ],
        "[execution succeeded]",
    );

    assert_eq!(
        output,
        "[compilation: echo compile >&2]\ncompile\necho run\nrun\n[execution succeeded]\n"
    );
}

#[test]
fn compile_command_error_output_is_not_printed_in_quiet_mode() {
    let dir = TestDir::new();
    dir.init_default_files();

    let output = run_until(
        dir.path(),
        &[
            "-q",
            "-m",
            "input.txt",
            "-r",
            "echo run",
            "--",
            "echo compile >&2",
        ],
        "[execution succeeded]",
    );

    assert_eq!(
        output,
        "[compilation: echo compile >&2]\necho run\nrun\n[execution succeeded]\n"
    );
}

#[test]
fn run_command_preserves_shell_syntax() {
    let dir = TestDir::new();
    dir.init_default_files();

    let output = run_until(
        dir.path(),
        &[
            "-m",
            "input.txt",
            "-r",
            r#"FOO=ok; echo "$FOO" | cat"#,
            "--",
            "echo compile",
        ],
        "[execution succeeded]",
    );

    assert_eq!(
        output,
        "[compilation: echo compile]\nFOO=ok; echo \"$FOO\" | cat\nok\n[execution succeeded]\n"
    );
}

#[test]
fn input_file_feeds_run_command_stdin() {
    let dir = TestDir::new();
    dir.init_default_files();
    dir.write("input.txt", "from input\n");

    let output = run_until(
        dir.path(),
        &["-i", "input.txt", "-r", "cat", "--", "echo compile"],
        "[execution succeeded]",
    );

    assert_eq!(
        output,
        "[compilation: echo compile]\ncat\nfrom input\n[execution succeeded]\n"
    );
}

#[test]
fn postpone_runs_after_file_change() {
    let dir = TestDir::new();
    dir.init_default_files();
    let mut inf = RunningInf::spawn(
        dir.path(),
        &[
            "-p",
            "-m",
            "input.txt",
            "-r",
            "echo run",
            "--",
            "echo compile",
        ],
    );

    assert_line_count_stays(&inf, "run", 0, Duration::from_millis(250));

    let output = trigger_until_line_count(
        &inf,
        &dir.path().join("input.txt"),
        "run",
        1,
        Duration::from_secs(5),
    )
    .unwrap_or_else(|err| panic!("postpone run was not triggered: {err}"));
    inf.stop();

    assert!(output.contains("[compilation: echo compile]"));
    assert_eq!(exact_line_count(&output, "run"), 1);
}

#[test]
fn multiple_monitor_flags_gather_all_files() {
    let dir = TestDir::new();
    dir.init_default_files();
    let mut inf = RunningInf::spawn(
        dir.path(),
        &[
            "-m",
            "input.txt",
            "-m",
            "extra.txt",
            "-r",
            "echo run",
            "--",
            "cat main.c >&2",
        ],
    );

    inf.wait_for_line_count_at_least("run", 1, Duration::from_secs(5))
        .expect("initial run");

    let output = trigger_until_line_count(
        &inf,
        &dir.path().join("input.txt"),
        "run",
        2,
        Duration::from_secs(5),
    )
    .expect("input trigger");
    assert_eq!(exact_line_count(&output, "run"), 2);

    let output = trigger_until_line_count(
        &inf,
        &dir.path().join("extra.txt"),
        "run",
        3,
        Duration::from_secs(5),
    )
    .expect("extra trigger");
    assert_eq!(exact_line_count(&output, "run"), 3);

    dir.write("dummy.txt", "dummy content\n");
    assert_line_count_stays(&inf, "run", 3, Duration::from_millis(250));

    dir.write("main.c", "maintext\n");
    let output = trigger_until_line_count(
        &inf,
        &dir.path().join("main.c"),
        "run",
        4,
        Duration::from_secs(5),
    )
    .expect("compile file trigger");
    inf.stop();

    assert_eq!(exact_line_count(&output, "run"), 4);
    assert!(output.contains("maintext"));
}

#[test]
fn compile_failure_prevents_run() {
    let dir = TestDir::new();
    dir.init_default_files();

    let mut inf = RunningInf::spawn(
        dir.path(),
        &["-m", "input.txt", "-r", "echo run", "--", "false"],
    );

    inf.wait_for_line_count_at_least("[compilation: false]", 1, Duration::from_secs(5))
        .expect("compilation header");

    std::thread::sleep(Duration::from_millis(200));
    let output = inf.output();
    inf.stop();

    assert!(!output.contains("run"));
}

#[test]
fn run_failure_prints_exit_code() {
    let dir = TestDir::new();
    dir.init_default_files();

    let output = run_until(
        dir.path(),
        &["-m", "input.txt", "-r", "exit 42", "--", "echo compile"],
        "[exit code = 42]",
    );

    assert!(output.contains("[exit code = 42]"));
}

#[test]
fn compile_only_prints_success() {
    let dir = TestDir::new();
    dir.init_default_files();

    let output = run_until(
        dir.path(),
        &["-m", "input.txt", "--", "echo compile"],
        "Compilation succeeded!",
    );

    assert!(output.contains("Compilation succeeded!"));
    assert!(!output.contains("[execution"));
}

#[test]
fn tilde_expansion_in_monitor_path() {
    let dir = TestDir::new();
    let home = std::env::var("HOME").expect("HOME set");
    let file = format!("{home}/.inf_test_tilde_expansion");
    std::fs::write(&file, "").expect("create test file");

    let result = std::process::Command::new(support::inf_bin())
        .args(["-d", "-m", "~/.inf_test_tilde_expansion", "--", "echo hi"])
        .current_dir(dir.path())
        .output()
        .expect("run inf");

    std::fs::remove_file(&file).ok();

    let stdout = String::from_utf8(result.stdout).expect("utf8");
    assert!(stdout.contains(".inf_test_tilde_expansion"));
}

#[test]
fn stdin_piped_monitor_files() {
    let dir = TestDir::new();
    dir.init_default_files();

    let output = std::process::Command::new(support::inf_bin())
        .args(["-d", "--", "echo hi"])
        .current_dir(dir.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"main.c\nextra.txt\n")
                .unwrap();
            child.wait_with_output()
        })
        .expect("run inf");

    let stdout = String::from_utf8(output.stdout).expect("utf8");
    let monitor_line = stdout
        .lines()
        .find(|line| line.contains("monitorFiles[*]"))
        .expect("monitor files line");
    assert!(monitor_line.contains("main.c"));
    assert!(monitor_line.contains("extra.txt"));
}
