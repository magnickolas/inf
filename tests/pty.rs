mod support;

use std::time::Duration;

use support::{PtyInf, TestDir, trigger_until_new_pid};

#[test]
#[ignore]
fn foreground_stdin_no_race() {
    let dir = TestDir::new();
    dir.write_interactive_app();

    for i in 0..50 {
        let mut inf = PtyInf::spawn(
            dir.path(),
            &["-r", "bash -c ./app", "--", "gcc main.c -o app"],
        );
        inf.wait_for_pid(Duration::from_secs(8))
            .unwrap_or_else(|e| panic!("iteration {i}: no pid: {e}"));

        let value = 1000 + i;
        inf.write(&format!("{value}\n"));
        inf.wait_for_contains(&format!("x={value}"), Duration::from_secs(5))
            .unwrap_or_else(|e| panic!("iteration {i}: expected x={value}: {e}"));
    }
}

#[test]
fn refresh_restarts_shell_interactive_command() {
    let dir = TestDir::new();
    dir.write_interactive_app();

    let mut inf = PtyInf::spawn(
        dir.path(),
        &["-x", "-r", "bash -c ./app", "--", "gcc main.c -o app"],
    );
    let first_pid = inf.wait_for_pid(Duration::from_secs(8)).expect("first pid");
    let second_pid = trigger_until_new_pid(
        &inf,
        &dir.path().join("main.c"),
        first_pid,
        Duration::from_secs(8),
    )
    .expect("second pid");

    assert_ne!(first_pid, second_pid);

    inf.write("37\n");
    inf.wait_for_contains("x=37", Duration::from_secs(5))
        .expect("interactive output");
}

#[test]
fn interactive_command_without_refresh_is_not_killed_on_change() {
    let dir = TestDir::new();
    dir.write_interactive_app();

    let mut inf = PtyInf::spawn(
        dir.path(),
        &["-r", "bash -c ./app", "--", "gcc main.c -o app"],
    );
    let first_pid = inf.wait_for_pid(Duration::from_secs(8)).expect("first pid");

    dir.append("main.c", "\n/* queue rerun */\n");
    inf.assert_only_pid_for(first_pid, Duration::from_secs(1));

    inf.write("12\n");
    inf.wait_for_contains("x=12", Duration::from_secs(5))
        .expect("original command accepts stdin");

    let second_pid = inf
        .wait_for_new_pid(first_pid, Duration::from_secs(8))
        .expect("queued rerun starts");
    assert_ne!(first_pid, second_pid);
}

#[test]
fn refresh_kills_shell_descendants() {
    let dir = TestDir::new();
    dir.write_interactive_app();

    let mut inf = PtyInf::spawn(
        dir.path(),
        &["-x", "-r", "sh -c 'sh -c ./app'", "--", "gcc main.c -o app"],
    );
    let first_pid = inf.wait_for_pid(Duration::from_secs(8)).expect("first pid");
    let second_pid = trigger_until_new_pid(
        &inf,
        &dir.path().join("main.c"),
        first_pid,
        Duration::from_secs(8),
    )
    .expect("second pid");

    assert_ne!(first_pid, second_pid);

    inf.write("44\n");
    inf.wait_for_contains("x=44", Duration::from_secs(5))
        .expect("interactive output");
}

#[test]
fn refresh_kills_descendant_that_ignores_sigterm() {
    use nix::{
        errno::Errno,
        sys::signal::{Signal, kill},
        unistd::Pid,
    };
    use std::time::Instant;

    let dir = TestDir::new();
    dir.touch("input.txt");
    let mut inf = PtyInf::spawn(
        dir.path(),
        &[
            "-x",
            "-m",
            "input.txt",
            "-r",
            "bash -c 'trap \"\" TERM; echo pid=$$; sleep 10' & wait",
        ],
    );
    let first_pid = inf.wait_for_pid(Duration::from_secs(5)).unwrap();
    let restarted = trigger_until_new_pid(
        &inf,
        &dir.path().join("input.txt"),
        first_pid,
        Duration::from_secs(5),
    );
    let pid = Pid::from_raw(first_pid as i32);
    let deadline = Instant::now() + Duration::from_secs(2);
    while kill(pid, None).is_ok() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let gone = kill(pid, None) == Err(Errno::ESRCH);
    kill(pid, Signal::SIGKILL).ok();
    inf.stop();
    restarted.expect("replacement command starts");
    assert!(gone, "old descendant survived refresh");
}

#[test]
fn waitkey_accepts_input_and_ctrl_c_at_next_prompt() {
    let dir = TestDir::new();
    dir.touch("input.txt");
    let mut inf = PtyInf::spawn(dir.path(), &["-w", "-m", "input.txt", "--", "true"]);
    inf.wait_for_contains("<press Enter to run>", Duration::from_secs(5))
        .unwrap();
    inf.write("\n");
    inf.wait_for_contains("Compilation succeeded!", Duration::from_secs(5))
        .unwrap();
    dir.append("input.txt", "change");
    inf.wait_for_contains("\r\n<press Enter to run>", Duration::from_secs(5))
        .unwrap();
    inf.write("\x03");
    let exited = inf.wait_for_exit(Duration::from_secs(2));
    inf.stop();
    assert!(exited, "Ctrl+C hung at the wait-key prompt");
}

#[test]
fn waitkey_consumes_the_whole_line_before_running() {
    let dir = TestDir::new();
    dir.touch("input.txt");
    let mut inf = PtyInf::spawn(
        dir.path(),
        &[
            "-w",
            "-m",
            "input.txt",
            "-r",
            "read -r value; echo value=$value",
        ],
    );
    inf.wait_for_contains("<press Enter to run>", Duration::from_secs(5))
        .unwrap();
    inf.write("abc\n");
    inf.wait_for_contains("read -r value; echo value=$value", Duration::from_secs(5))
        .unwrap();
    // Text entered at the prompt must not become the target's input.
    std::thread::sleep(Duration::from_millis(250));
    assert!(!inf.output().contains("value=bc"), "{}", inf.output());
    inf.write("target input\n");
    inf.wait_for_contains("value=target input", Duration::from_secs(5))
        .unwrap();
    inf.wait_for_contains("[execution succeeded]", Duration::from_secs(5))
        .unwrap();
    dir.append("input.txt", "change");
    inf.wait_for_contains("\r\n<press Enter to run>", Duration::from_secs(5))
        .unwrap();
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(inf.output().matches("<press Enter to run>").count(), 2);
    assert_eq!(inf.output().matches("[execution succeeded]").count(), 1);
    inf.stop();
}

#[test]
fn ctrl_z_ends_the_stopped_run_and_watching_continues() {
    let dir = TestDir::new();
    dir.touch("input.txt");
    let mut inf = PtyInf::spawn(
        dir.path(),
        &["-z", "-m", "input.txt", "-r", "echo pid=$$; read -r value"],
    );
    let timeout = Duration::from_secs(5);
    let first_pid = inf.wait_for_pid(timeout).unwrap();

    inf.write("\x1a");
    inf.wait_for_contains("ending this run", timeout).unwrap();
    dir.append("input.txt", "change");
    inf.wait_for_new_pid(first_pid, timeout).unwrap();
    inf.write("continue\n");
    inf.stop();
}

#[test]
fn background_watcher_leaves_the_shell_in_the_foreground() {
    use std::io::{BufRead, BufReader};

    use portable_pty::{CommandBuilder, PtySize, native_pty_system};

    let dir = TestDir::new();
    dir.touch("source");
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new("bash");
    command.cwd(dir.path());
    command.env("INF_TEST_BIN", support::inf_bin());
    command.args([
        "-c",
        r#"
            set -m
            before=$(ps -o tpgid= -p "$$")
            "$INF_TEST_BIN" -z -m source -r 'touch ready; sleep 5' &
            watcher=$!
            trap 'kill -TERM "$watcher" 2>/dev/null; wait "$watcher" 2>/dev/null' EXIT

            # Stay in the shell's group while waiting for the target to start.
            until [[ -f ready || $SECONDS -ge 4 ]]; do :; done
            [[ -f ready ]] || exit 1
            group=$(ps -o pgid= -p "$watcher")
            after=$(ps -o tpgid= -p "$$")
            printf 'shell=%s watcher=%s after=%s\n' "$before" "$group" "$after"
            [[ $before != "$group" && $before == "$after" ]]
        "#,
    ]);
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let reader = BufReader::new(pair.master.try_clone_reader().unwrap());
    let output = reader
        .lines()
        .map_while(Result::ok)
        .find(|line| line.starts_with("shell="));
    assert!(child.wait().unwrap().success(), "{output:?}");
}

#[test]
fn default_compiler_capture_allows_terminal_input() {
    let dir = TestDir::new();
    dir.touch("source");
    let mut inf = PtyInf::spawn(
        dir.path(),
        &[
            "-z",
            "-m",
            "source",
            "-r",
            "echo target-ran",
            "--",
            "echo compiler-ready >&2; read -r value; echo captured; echo accepted:$value >&2",
        ],
    );
    let timeout = Duration::from_secs(5);
    inf.wait_for_contains("compiler-ready\r\n", timeout)
        .unwrap();
    inf.write("answer\n");
    inf.wait_for_contains("accepted:answer", timeout).unwrap();
    let output = inf.wait_for_contains("target-ran", timeout).unwrap();
    assert!(!output.contains("captured"));
}

#[test]
fn waitkey_uses_terminal_after_reading_piped_monitor_paths() {
    let dir = TestDir::new();
    dir.touch("source");
    let mut command = portable_pty::CommandBuilder::new("bash");
    command.cwd(dir.path());
    command.env("INF_TEST_BIN", support::inf_bin());
    command.args([
        "-c",
        r#"printf 'source\n' | "$INF_TEST_BIN" -w -r 'echo target-ran'"#,
    ]);
    let mut inf = PtyInf::spawn_command(command);
    let timeout = Duration::from_secs(5);
    inf.wait_for_contains("<press Enter to run>", timeout)
        .unwrap();
    inf.write("\n");
    inf.wait_for_contains("[execution succeeded]", timeout)
        .unwrap();
    inf.write("\x03");
    assert!(inf.wait_for_exit(timeout));
}
