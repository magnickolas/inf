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
fn quiet_mode_suppresses_failed_compiler_output() {
    use std::time::Instant;

    let dir = TestDir::new();
    dir.init_default_files();
    let mut inf = RunningInf::spawn(
        dir.path(),
        &[
            "-q",
            "-z",
            "-m",
            "input.txt",
            "-r",
            "echo run",
            "--",
            "echo OUT; echo ERR >&2; touch done; exit 1",
        ],
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !dir.path().join("done").is_file() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(dir.path().join("done").is_file(), "compiler ran");
    std::thread::sleep(Duration::from_millis(200));
    inf.stop();
    assert_eq!(
        inf.output(),
        "",
        "failed compiler is silent and target does not run"
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
fn deleted_input_file_can_be_recreated_without_restarting_inf() {
    let dir = TestDir::new();
    dir.write("input.txt", "original\n");
    let mut inf = RunningInf::spawn(dir.path(), &["-i", "input.txt", "-r", "cat"]);
    let timeout = Duration::from_secs(5);
    inf.wait_for_line_count_at_least("original", 1, timeout)
        .unwrap();

    let path = dir.path().join("input.txt");
    std::fs::remove_file(&path).unwrap();
    let error = std::fs::File::open(path).unwrap_err();
    inf.wait_for_line_count_at_least(
        &format!("ERROR: failed to open input file \"input.txt\": {error}"),
        1,
        timeout,
    )
    .unwrap();

    dir.write("input.txt", "recreated\n");
    inf.wait_for_line_count_at_least("recreated", 1, timeout)
        .unwrap();
    inf.stop();
}

#[test]
fn fifo_replacement_is_rejected_without_blocking_watcher() {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let dir = TestDir::new();
    dir.write("input.txt", "original\n");
    let mut inf = RunningInf::spawn(dir.path(), &["-i", "input.txt", "-r", "cat"]);
    let timeout = Duration::from_secs(5);
    inf.wait_for_line_count_at_least("original", 1, timeout)
        .unwrap();

    let fifo = dir.path().join("fifo");
    let fifo_name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: fifo_name is a valid NUL-terminated path in the test directory.
    assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
    std::fs::rename(fifo, dir.path().join("input.txt")).unwrap();
    inf.wait_for_line_count_at_least(
        "ERROR: failed to open input file \"input.txt\": expected a regular file",
        1,
        timeout,
    )
    .unwrap();
    assert!(inf.interrupt_and_wait(timeout));
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
fn same_size_edit_within_one_second_triggers_run() {
    use std::{
        fs::{File, FileTimes},
        time::UNIX_EPOCH,
    };

    let dir = TestDir::new();
    dir.write("input.txt", "aaaa");
    let file = File::options()
        .write(true)
        .open(dir.path().join("input.txt"))
        .unwrap();
    let second = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    file.set_times(FileTimes::new().set_modified(second + Duration::from_millis(100)))
        .unwrap();
    let mut inf = RunningInf::spawn(dir.path(), &["-m", "input.txt", "-r", "echo run"]);
    inf.wait_for_line_count_at_least("[execution succeeded]", 1, Duration::from_secs(5))
        .unwrap();

    dir.write("input.txt", "bbbb");
    file.set_times(FileTimes::new().set_modified(second + Duration::from_millis(200)))
        .unwrap();
    inf.wait_for_line_count_at_least("run", 2, Duration::from_secs(5))
        .expect("same-size edit triggers another run");
    inf.stop();
}

#[test]
fn same_size_edit_with_preserved_mtime_triggers_run() {
    use std::fs::{File, FileTimes};

    let dir = TestDir::new();
    dir.write("input.txt", "aaaa\n");
    let path = dir.path().join("input.txt");
    let modified = path.metadata().unwrap().modified().unwrap();
    let mut inf = RunningInf::spawn(dir.path(), &["-m", "input.txt", "-r", "cat input.txt"]);
    inf.wait_for_line_count_at_least("[execution succeeded]", 1, Duration::from_secs(5))
        .unwrap();

    dir.write("input.txt", "bbbb\n");
    File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    inf.wait_for_line_count_at_least("bbbb", 1, Duration::from_secs(5))
        .expect("content change with preserved mtime triggers a run");
    inf.stop();
}

#[test]
fn autodetection_does_not_watch_compiler_output() {
    let dir = TestDir::new();
    dir.write("main.c", "int main(void) { return 0; }\n");
    dir.touch("app.bin");
    dir.touch("deps.d");
    let mut inf = RunningInf::spawn(
        dir.path(),
        &["cc", "-MMD", "-MF", "deps.d", "-o", "app.bin", "main.c"],
    );
    let timeout = Duration::from_secs(5);
    let success = "Compilation succeeded!";
    inf.wait_for_line_count_at_least(success, 1, timeout)
        .unwrap();
    assert_line_count_stays(&inf, success, 1, Duration::from_millis(800));

    dir.append("main.c", "/* trigger a rebuild */\n");
    inf.wait_for_line_count_at_least(success, 2, timeout)
        .unwrap();
    assert_line_count_stays(&inf, success, 2, Duration::from_millis(800));
    inf.stop();
}

#[test]
fn autodetects_compile_files_with_dot_relative_paths() {
    let dir = TestDir::new();
    dir.write("main.c", "original\n");
    let mut inf = RunningInf::spawn(dir.path(), &["-v", "--", "cat ./main.c"]);
    let timeout = Duration::from_secs(5);
    inf.wait_for_line_count_at_least("original", 1, timeout)
        .unwrap();
    dir.write("main.c", "updated\n");
    inf.wait_for_line_count_at_least("updated", 1, timeout)
        .unwrap();
    inf.stop();
}

#[test]
fn autodetected_path_with_parent_component_triggers_run() {
    let dir = TestDir::new();
    dir.init_default_files();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    let mut inf = RunningInf::spawn(dir.path(), &["-r", "echo run", "--", "cat sub/../main.c"]);
    inf.wait_for_line_count_at_least("[execution succeeded]", 1, Duration::from_secs(5))
        .unwrap();
    dir.append("main.c", "change");
    inf.wait_for_line_count_at_least("run", 2, Duration::from_secs(5))
        .expect("autodetected path matches canonical filesystem events");
    inf.stop();
}

#[test]
fn replaced_symlink_tracks_new_target_in_another_directory() {
    use std::os::unix::fs::symlink;

    let dir = TestDir::new();
    std::fs::create_dir(dir.path().join("old")).unwrap();
    std::fs::create_dir(dir.path().join("new")).unwrap();
    dir.write("old/source.txt", "old\n");
    dir.write("new/source.txt", "new\n");
    symlink("old/source.txt", dir.path().join("link.txt")).unwrap();
    let mut inf = RunningInf::spawn(dir.path(), &["-m", "link.txt", "-r", "cat link.txt"]);
    inf.wait_for_line_count_at_least("old", 1, Duration::from_secs(5))
        .unwrap();

    dir.write("old/source.txt", "old edited\n");
    inf.wait_for_line_count_at_least("old edited", 1, Duration::from_secs(5))
        .unwrap();
    symlink("new/source.txt", dir.path().join("replacement")).unwrap();
    std::fs::rename(dir.path().join("replacement"), dir.path().join("link.txt")).unwrap();
    inf.wait_for_line_count_at_least("new", 1, Duration::from_secs(5))
        .expect("symlink replacement triggers a run");
    dir.write("new/source.txt", "new edited\n");
    inf.wait_for_line_count_at_least("new edited", 1, Duration::from_secs(5))
        .expect("new target is watched");
    inf.wait_for_line_count_at_least("[execution succeeded]", 4, Duration::from_secs(5))
        .unwrap();
    dir.write("old/source.txt", "old changed again\n");
    assert_line_count_stays(&inf, "[execution succeeded]", 4, Duration::from_millis(500));
    std::fs::remove_file(dir.path().join("new/source.txt")).unwrap();
    inf.wait_for_line_count_at_least("[exit code = 1]", 1, Duration::from_secs(5))
        .expect("target deletion triggers a run");
    dir.write("new/source.txt", "recreated\n");
    inf.wait_for_line_count_at_least("recreated", 1, Duration::from_secs(5))
        .expect("target recreation is still watched");
    inf.stop();
}

#[test]
fn replaced_directory_symlink_tracks_files_in_the_new_target() {
    use std::os::unix::fs::symlink;

    let dir = TestDir::new();
    std::fs::create_dir(dir.path().join("old")).unwrap();
    std::fs::create_dir(dir.path().join("new")).unwrap();
    dir.write("old/source.txt", "old\n");
    dir.write("new/source.txt", "new\n");
    symlink("old", dir.path().join("current")).unwrap();
    let mut inf = RunningInf::spawn(
        dir.path(),
        &["-m", "current/source.txt", "-r", "cat current/source.txt"],
    );
    let timeout = Duration::from_secs(5);
    inf.wait_for_line_count_at_least("old", 1, timeout).unwrap();

    symlink("new", dir.path().join("replacement")).unwrap();
    std::fs::rename(dir.path().join("replacement"), dir.path().join("current")).unwrap();
    inf.wait_for_line_count_at_least("new", 1, timeout)
        .expect("directory symlink replacement triggers a run");
    dir.write("new/source.txt", "new edited\n");
    inf.wait_for_line_count_at_least("new edited", 1, timeout)
        .expect("new target is watched");
    inf.stop();
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
fn interrupt_cancels_capture_after_shell_exits() {
    use nix::{
        sys::signal::{Signal, kill},
        unistd::Pid,
    };

    let dir = TestDir::new();
    dir.init_default_files();
    let mut inf = RunningInf::spawn(
        dir.path(),
        &[
            "-m",
            "input.txt",
            "--",
            "sleep 10 & echo $! > sleeper.pid; echo READY >&2; exit 0",
        ],
    );
    inf.wait_for_line_count_at_least("READY", 1, Duration::from_secs(5))
        .unwrap();
    // Let the shell exit while its background child retains the output pipe.
    std::thread::sleep(Duration::from_millis(200));
    let exited = inf.interrupt_and_wait(Duration::from_secs(2));
    let pid: i32 = std::fs::read_to_string(dir.path().join("sleeper.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    kill(Pid::from_raw(pid), Signal::SIGKILL).ok();
    inf.stop();
    assert!(exited, "interrupt hung waiting for captured output");
}

#[test]
fn background_descendant_does_not_survive_shell_completion() {
    use nix::{
        errno::Errno,
        sys::signal::{Signal, kill},
        unistd::Pid,
    };
    use std::time::Instant;

    let dir = TestDir::new();
    dir.touch("input.txt");
    let mut inf = RunningInf::spawn(
        dir.path(),
        &["-m", "input.txt", "-r", "sleep 30 & echo $! > sleeper.pid"],
    );
    inf.wait_for_line_count_at_least("[execution succeeded]", 1, Duration::from_secs(5))
        .unwrap();
    let pid = Pid::from_raw(
        std::fs::read_to_string(dir.path().join("sleeper.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while kill(pid, None).is_ok() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let gone = kill(pid, None) == Err(Errno::ESRCH);
    kill(pid, Signal::SIGKILL).ok();
    inf.stop();
    assert!(gone, "background descendant survived its shell");
}

#[test]
fn run_terminated_by_signal_prints_signal() {
    let dir = TestDir::new();
    dir.touch("source");
    let output = run_until(
        dir.path(),
        &["-z", "-m", "source", "-r", "kill -TERM $$"],
        "[terminated by SIGTERM]",
    );
    assert_eq!(output, "[terminated by SIGTERM]\n");
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

#[test]
fn blocked_compiler_output_replay_can_be_interrupted() {
    use std::{
        os::fd::AsRawFd,
        process::{Command, Stdio},
        thread,
        time::Instant,
    };

    use nix::{
        sys::signal::{Signal, kill},
        unistd::Pid,
    };

    let dir = TestDir::new();
    dir.touch("source");
    let mut child = Command::new(support::inf_bin())
        .current_dir(dir.path())
        .args([
            "-z",
            "-m",
            "source",
            "--",
            "head -c 1048576 /dev/zero; exit 1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let mut stdout = libc::pollfd {
        fd: child.stdout.as_ref().unwrap().as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: stdout describes one open pipe descriptor owned by child.stdout.
    let ready = unsafe { libc::poll(&mut stdout, 1, 5000) };
    assert_ne!(stdout.revents & libc::POLLIN, 0, "poll returned {ready}");
    thread::sleep(Duration::from_millis(200));

    kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    panic!("shutdown waited for the stdout pipe to be drained");
}

#[test]
fn invalid_tmpdir_does_not_prevent_compilation() {
    let dir = TestDir::new();
    dir.touch("source");
    assert_cmd::Command::new(support::inf_bin())
        .current_dir(dir.path())
        .env("TMPDIR", dir.path().join("missing"))
        .args([
            "-z",
            "-m",
            "source",
            "--",
            "touch compiled; kill -TERM \"$PPID\"",
        ])
        .timeout(Duration::from_secs(5))
        .assert()
        .success()
        .stdout("");
    assert!(dir.path().join("compiled").is_file());
}

#[test]
fn compiler_arguments_preserve_spaces_and_metacharacters() {
    let dir = TestDir::new();
    dir.write("main file;.txt", "source contents\n");
    let output = run_until(
        dir.path(),
        &["-v", "--", "cat", "main file;.txt"],
        "Compilation succeeded!",
    );
    assert!(output.contains("\nsource contents\n"), "{output}");
}
