#![allow(dead_code)]

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use assert_cmd::cargo::cargo_bin;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tempfile::TempDir;

#[cfg(unix)]
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};

pub struct TestDir {
    _temp: TempDir,
    path: PathBuf,
}

impl TestDir {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().expect("create temp dir");
        let path = temp.path().to_path_buf();
        Self { _temp: temp, path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn touch(&self, path: &str) {
        self.write(path, "");
    }

    pub fn write(&self, path: &str, contents: &str) {
        fs::write(self.path.join(path), contents).expect("write test file");
    }

    pub fn append(&self, path: &str, contents: &str) {
        let mut file = OpenOptions::new()
            .append(true)
            .open(self.path.join(path))
            .expect("open test file for append");
        file.write_all(contents.as_bytes())
            .expect("append test file");
    }

    pub fn init_default_files(&self) {
        self.touch("main.c");
        self.touch("input.txt");
        self.touch("extra.txt");
    }

    pub fn write_interactive_app(&self) {
        self.write(
            "main.c",
            r#"#include <stdio.h>
#include <unistd.h>

int main(void) {
    int x;
    printf("pid=%d\n", (int)getpid());
    fflush(stdout);
    if (scanf("%d", &x) != 1) {
        return 2;
    }
    printf("x=%d\n", x);
    return 0;
}
"#,
        );
    }
}

pub fn inf_bin() -> PathBuf {
    cargo_bin("inf")
}

pub struct RunningInf {
    child: Child,
    log: PathBuf,
}

impl RunningInf {
    pub fn spawn(cwd: &Path, args: &[&str]) -> Self {
        let log = cwd.join("inf.log");
        let stdout = File::create(&log).expect("create inf log");
        let stderr = stdout.try_clone().expect("clone inf log");
        let child = Command::new(inf_bin())
            .args(args)
            .current_dir(cwd)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .expect("spawn inf");

        Self { child, log }
    }

    pub fn output(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }

    pub fn stop(&mut self) {
        if self.child.try_wait().expect("poll inf").is_some() {
            return;
        }

        interrupt_process(self.child.id());
        if !wait_for_child_exit(&mut self.child, Duration::from_secs(2)) {
            self.child.kill().ok();
            self.child.wait().ok();
        }
    }

    pub fn wait_for_line_count_at_least(
        &self,
        line: &str,
        count: usize,
        timeout: Duration,
    ) -> io::Result<String> {
        wait_until(timeout, || {
            let output = self.output();
            (exact_line_count(&output, line) >= count).then_some(output)
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, self.output()))
    }
}

impl Drop for RunningInf {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn trigger_until_line_count(
    run: &RunningInf,
    file: &Path,
    line: &str,
    count: usize,
    timeout: Duration,
) -> io::Result<String> {
    let start = Instant::now();
    loop {
        if start.elapsed() >= timeout {
            return Err(io::Error::new(io::ErrorKind::TimedOut, run.output()));
        }

        let mut file = OpenOptions::new().append(true).open(file)?;
        file.write_all(b".\n")?;

        if let Some(output) = wait_until(Duration::from_millis(500), || {
            let output = run.output();
            (exact_line_count(&output, line) >= count).then_some(output)
        }) {
            return Ok(output);
        }
    }
}

pub fn exact_line_count(output: &str, line: &str) -> usize {
    output.lines().filter(|actual| *actual == line).count()
}

pub fn assert_line_count_stays(run: &RunningInf, line: &str, count: usize, duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        let output = run.output();
        assert_eq!(exact_line_count(&output, line), count, "{output}");
        thread::sleep(Duration::from_millis(20));
    }
}

pub struct PtyInf {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    output: Arc<Mutex<String>>,
    reader: Option<thread::JoinHandle<()>>,
}

impl PtyInf {
    pub fn spawn(cwd: &Path, args: &[&str]) -> Self {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open pty");

        let bin = inf_bin();
        let mut command = CommandBuilder::new(bin.as_os_str());
        command.args(args);
        command.cwd(cwd.as_os_str());

        let child = pair.slave.spawn_command(command).expect("spawn inf in pty");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("clone pty reader");
        let writer = pair.master.take_writer().expect("take pty writer");
        let output = Arc::new(Mutex::new(String::new()));
        let reader_output = Arc::clone(&output);
        let reader = thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        reader_output
                            .lock()
                            .expect("lock pty output")
                            .push_str(&String::from_utf8_lossy(&buffer[..n]));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            child,
            writer,
            output,
            reader: Some(reader),
        }
    }

    pub fn write(&mut self, input: &str) {
        self.writer
            .write_all(input.as_bytes())
            .expect("write to pty");
        self.writer.flush().expect("flush pty");
    }

    pub fn output(&self) -> String {
        self.output.lock().expect("lock pty output").clone()
    }

    pub fn wait_for_contains(&self, needle: &str, timeout: Duration) -> io::Result<String> {
        wait_until(timeout, || {
            let output = self.output();
            output.contains(needle).then_some(output)
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, self.output()))
    }

    pub fn wait_for_pid(&self, timeout: Duration) -> io::Result<u32> {
        wait_until(timeout, || pids_from_output(&self.output()).last().copied())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, self.output()))
    }

    pub fn wait_for_new_pid(&self, old: u32, timeout: Duration) -> io::Result<u32> {
        wait_until(timeout, || {
            pids_from_output(&self.output())
                .into_iter()
                .find(|pid| *pid != old)
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, self.output()))
    }

    pub fn assert_only_pid_for(&self, pid: u32, duration: Duration) {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            assert_eq!(pids_from_output(&self.output()), vec![pid]);
            thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn stop(&mut self) {
        if self.child.try_wait().expect("poll pty child").is_none()
            && let Some(pid) = self.child.process_id()
        {
            interrupt_process(pid);
        }

        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if self.child.try_wait().expect("poll pty child").is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        if self.child.try_wait().expect("poll pty child").is_none() {
            self.child.kill().ok();
            self.child.wait().ok();
        }

        if let Some(reader) = self.reader.take() {
            reader.join().ok();
        }
    }
}

impl Drop for PtyInf {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn trigger_until_new_pid(
    run: &PtyInf,
    file: &Path,
    old_pid: u32,
    timeout: Duration,
) -> io::Result<u32> {
    let start = Instant::now();
    loop {
        if start.elapsed() >= timeout {
            return Err(io::Error::new(io::ErrorKind::TimedOut, run.output()));
        }

        let mut file = OpenOptions::new().append(true).open(file)?;
        file.write_all(b"\n/* trigger */\n")?;

        if let Ok(pid) = run.wait_for_new_pid(old_pid, Duration::from_millis(500)) {
            return Ok(pid);
        }
    }
}

fn pids_from_output(output: &str) -> Vec<u32> {
    let mut pids = Vec::new();
    let mut rest = output;
    while let Some(index) = rest.find("pid=") {
        let digits = rest[index + 4..]
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>();
        if let Ok(pid) = digits.parse() {
            pids.push(pid);
        }
        rest = &rest[index + 4..];
    }
    pids
}

fn wait_until<T>(timeout: Duration, mut predicate: impl FnMut() -> Option<T>) -> Option<T> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = predicate() {
            return Some(value);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> bool {
    wait_until(timeout, || child.try_wait().ok().flatten()).is_some()
}

#[cfg(unix)]
fn interrupt_process(pid: u32) {
    kill(Pid::from_raw(pid as i32), Signal::SIGINT).ok();
}

#[cfg(not(unix))]
fn interrupt_process(_pid: u32) {}
