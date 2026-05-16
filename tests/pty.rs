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
