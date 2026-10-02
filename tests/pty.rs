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
