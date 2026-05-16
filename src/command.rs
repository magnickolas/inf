use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    process::ExitStatus,
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use anyhow::Context;
use anyhow::Result;
use command_group::{AsyncCommandGroup, AsyncGroupChild};
use nix::{
    errno::Errno,
    sys::{
        signal::{self, SaFlags, SigAction, SigHandler, SigSet, Signal, killpg},
        termios::{self, SetArg, Termios},
    },
    unistd::{Pid, tcgetpgrp, tcsetpgrp},
};
use tokio::sync::watch;
use tokio::{io::AsyncReadExt, process::Command, time};

use crate::{config::Config, output::Colors};

const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(20);
const TERMINATE_GRACE_PERIOD: Duration = Duration::from_millis(500);

#[derive(Debug)]
pub enum RunResult {
    Completed,
    Cancelled,
}

pub async fn compile_and_run(
    config: Arc<Config>,
    colors: Arc<Colors>,
    mut cancel: watch::Receiver<u64>,
) -> Result<RunResult> {
    if config.waitkey {
        wait_for_key()?;
    }

    if config.clear_screen {
        print!("\x1b[2J\x1b[H");
        io::stdout().flush()?;
    }

    if config.compile_cmd.is_some() && !config.zen {
        println!(
            "{}[compilation: {}]{}",
            colors.blue,
            config.compile_cmd.as_deref().unwrap_or(""),
            colors.reset
        );
    }

    if config.run_cmd.is_some() {
        if config.compile_cmd.is_some() {
            let compile_status = run_compile(&config, &mut cancel).await?;
            match compile_status {
                CommandResult::Cancelled => return Ok(RunResult::Cancelled),
                CommandResult::Finished(status) if !status.success() => {
                    return Ok(RunResult::Completed);
                }
                CommandResult::Finished(_) => {}
            }
        }

        match run_target(&config, &colors, &mut cancel).await? {
            CommandResult::Cancelled => Ok(RunResult::Cancelled),
            CommandResult::Finished(_) => Ok(RunResult::Completed),
        }
    } else if config.compile_cmd.is_some() {
        match run_compile(&config, &mut cancel).await? {
            CommandResult::Cancelled => Ok(RunResult::Cancelled),
            CommandResult::Finished(status) => {
                if status.success() && !config.zen {
                    println!("{}Compilation succeeded!{}", colors.green, colors.reset);
                }
                Ok(RunResult::Completed)
            }
        }
    } else {
        Ok(RunResult::Completed)
    }
}

enum CommandResult {
    Finished(ExitStatus),
    Cancelled,
}

async fn run_compile(config: &Config, cancel: &mut watch::Receiver<u64>) -> Result<CommandResult> {
    let command = config.compile_cmd.as_deref().unwrap_or("");

    if config.verbose {
        // Verbose mode is intentionally not captured: the old Bash wrapper let
        // compilers stream progress, colors, prompts, and diagnostics directly
        // to the terminal in this mode. Treat it like a foreground command so
        // tools that inspect their stdout/stderr fds see the same environment.
        return run_shell(command, None, OutputMode::Inherit, cancel).await;
    }

    let mode = if config.quiet {
        OutputMode::CaptureBoth
    } else {
        OutputMode::CaptureStdout
    };
    let result = run_shell(command, None, mode, cancel).await?;
    Ok(result)
}

async fn run_target(
    config: &Config,
    colors: &Colors,
    cancel: &mut watch::Receiver<u64>,
) -> Result<CommandResult> {
    let command = config.run_cmd.as_deref().unwrap_or("");
    if !config.zen {
        println!("{}{}{}", colors.green, command, colors.reset);
    }

    let result = run_shell(
        command,
        config.input_file.as_deref(),
        OutputMode::Inherit,
        cancel,
    )
    .await?;
    if let CommandResult::Finished(status) = &result {
        if !status.success() {
            println!(
                "{}[exit code = {}]{}",
                colors.red,
                status.code().unwrap_or(1),
                colors.reset
            );
        } else if !config.zen {
            println!("{}[execution succeeded]{}", colors.green, colors.reset);
        }
    }
    Ok(result)
}

#[derive(Clone, Copy)]
enum OutputMode {
    Inherit,
    CaptureStdout,
    CaptureBoth,
}

async fn run_shell(
    command: &str,
    input_file: Option<&str>,
    mode: OutputMode,
    cancel: &mut watch::Receiver<u64>,
) -> Result<CommandResult> {
    match mode {
        OutputMode::Inherit => {
            let cmd = shell_command(command, input_file, mode)?;
            run_in_foreground(cmd, cancel).await
        }
        OutputMode::CaptureStdout | OutputMode::CaptureBoth => {
            let cmd = shell_command(command, input_file, mode)?;
            run_captured(cmd, mode, cancel).await
        }
    }
}

fn shell_command(command: &str, input_file: Option<&str>, mode: OutputMode) -> Result<Command> {
    let mut cmd = Command::new("bash");
    cmd.arg("-c").arg(command);

    match mode {
        OutputMode::Inherit => {
            cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        }
        OutputMode::CaptureStdout => {
            cmd.stdout(Stdio::piped()).stderr(Stdio::inherit());
        }
        OutputMode::CaptureBoth => {
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        }
    }

    if let Some(input_file) = input_file {
        let file = File::open(input_file)
            .with_context(|| format!("failed to open input file {input_file:?}"))?;
        cmd.stdin(Stdio::from(file));
    }

    Ok(cmd)
}

async fn run_in_foreground(
    cmd: Command,
    cancel: &mut watch::Receiver<u64>,
) -> Result<CommandResult> {
    let version = *cancel.borrow();
    let mut child = spawn_foreground(cmd)?;
    wait_for_foreground_child(&mut child, cancel, version).await
}

async fn run_captured(
    cmd: Command,
    mode: OutputMode,
    cancel: &mut watch::Receiver<u64>,
) -> Result<CommandResult> {
    let version = *cancel.borrow();
    // Captured commands are not interactive, so they do not need terminal
    // foreground ownership. They still run in a process group so cancellation
    // can stop descendant processes.
    let mut group = ChildGroup::spawn(cmd)?;
    let mut stdout = group.inner_mut().stdout.take();
    let mut stderr = group.inner_mut().stderr.take();
    let stdout_task = tokio::spawn(async move {
        let mut output = Vec::new();
        if let Some(out) = stdout.as_mut() {
            out.read_to_end(&mut output).await?;
        }
        Ok::<_, io::Error>(output)
    });
    let stderr_task = tokio::spawn(async move {
        let mut output = Vec::new();
        if let Some(err) = stderr.as_mut() {
            err.read_to_end(&mut output).await?;
        }
        Ok::<_, io::Error>(output)
    });

    tokio::select! {
        status = group.wait() => {
            let status = status?;
            let stdout = stdout_task.await??;
            let stderr = stderr_task.await??;
            if !status.success() {
                io::stdout().write_all(&stdout)?;
                if matches!(mode, OutputMode::CaptureBoth) {
                    io::stdout().write_all(&stderr)?;
                }
                io::stdout().flush()?;
            }
            Ok(CommandResult::Finished(status))
        }
        _ = wait_for_cancel(cancel, version) => {
            group.kill().await;
            Ok(CommandResult::Cancelled)
        }
    }
}

async fn wait_for_foreground_child(
    child: &mut ForegroundChild,
    cancel: &mut watch::Receiver<u64>,
    version: u64,
) -> Result<CommandResult> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(CommandResult::Finished(status));
        }

        tokio::select! {
            _ = wait_for_cancel(cancel, version) => {
                child.restore_terminal();
                child.terminate_gracefully().await?;
                return Ok(CommandResult::Cancelled);
            }
            _ = time::sleep(CHILD_POLL_INTERVAL) => {}
        }
    }
}

fn spawn_foreground(mut cmd: Command) -> Result<ForegroundChild> {
    // The child must be in a separate process group so we can kill the whole
    // subtree, but a background group gets SIGTTIN on terminal reads. The child
    // claims terminal foreground itself in pre_exec — after setpgid but before
    // exec — so by the time bash starts there is no race window.
    let original_pgid = original_foreground_pgid();
    let original_termios = save_termios();

    // SAFETY: All syscalls (open, signal, tcsetpgrp, getpid, close) are
    // async-signal-safe per POSIX and valid between fork and exec.
    unsafe {
        cmd.pre_exec(move || {
            let tty_fd = libc::open(c"/dev/tty".as_ptr(), libc::O_RDWR);
            if tty_fd < 0 {
                return Ok(());
            }
            libc::signal(libc::SIGTTOU, libc::SIG_IGN);
            libc::tcsetpgrp(tty_fd, libc::getpid());
            libc::signal(libc::SIGTTOU, libc::SIG_DFL);
            libc::close(tty_fd);
            Ok(())
        });
    }

    cmd.process_group(0);
    let child = cmd.spawn().context("failed to spawn foreground command")?;

    let pgid = child.id().map(|id| Pid::from_raw(id as i32));
    Ok(ForegroundChild {
        child,
        pgid,
        original_pgid,
        original_termios,
    })
}

fn original_foreground_pgid() -> Option<Pid> {
    let tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    tcgetpgrp(&tty).ok()
}

fn save_termios() -> Option<Termios> {
    let tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    termios::tcgetattr(&tty).ok()
}

fn restore_termios(saved: &Termios) {
    let Ok(tty) = OpenOptions::new().read(true).write(true).open("/dev/tty") else {
        return;
    };
    let _ = termios::tcsetattr(&tty, SetArg::TCSANOW, saved);
}

struct ForegroundChild {
    child: tokio::process::Child,
    pgid: Option<Pid>,
    original_pgid: Option<Pid>,
    original_termios: Option<Termios>,
}

impl ForegroundChild {
    fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        Ok(self.child.try_wait()?)
    }

    fn restore_terminal(&mut self) {
        let Some(original) = self.original_pgid.take() else {
            return;
        };
        let Ok(tty) = OpenOptions::new().read(true).write(true).open("/dev/tty") else {
            return;
        };
        let _ = with_ignored_sigttou(|| {
            tcsetpgrp(&tty, original).context("failed to restore terminal foreground")
        });
        if let Some(termios) = self.original_termios.take() {
            restore_termios(&termios);
        }
    }

    async fn kill(&mut self) {
        if let Some(pgid) = self.pgid {
            ignore_no_process(killpg(pgid, Signal::SIGKILL)).ok();
        }
        self.child.kill().await.ok();
    }

    async fn terminate_gracefully(&mut self) -> Result<()> {
        if let Some(pgid) = self.pgid {
            ignore_no_process(killpg(pgid, Signal::SIGTERM))?;
        }

        let deadline = time::sleep(TERMINATE_GRACE_PERIOD);
        tokio::pin!(deadline);
        loop {
            if self.child.try_wait()?.is_some() {
                return Ok(());
            }
            tokio::select! {
                _ = &mut deadline => {
                    self.kill().await;
                    return Ok(());
                }
                _ = time::sleep(CHILD_POLL_INTERVAL) => {}
            }
        }
    }
}

impl Drop for ForegroundChild {
    fn drop(&mut self) {
        self.restore_terminal();
    }
}

struct ChildGroup {
    child: AsyncGroupChild,
}

impl ChildGroup {
    fn spawn(mut cmd: Command) -> Result<Self> {
        let child = cmd
            .group()
            .kill_on_drop(true)
            .spawn()
            .context("failed to spawn command")?;
        Ok(Self { child })
    }

    fn inner_mut(&mut self) -> &mut tokio::process::Child {
        self.child.inner()
    }

    async fn wait(&mut self) -> Result<ExitStatus> {
        Ok(self.child.wait().await?)
    }

    async fn kill(&mut self) {
        self.child.kill().await.ok();
    }
}

fn ignore_no_process(result: nix::Result<()>) -> Result<()> {
    match result {
        // The process group may disappear naturally between try_wait(), id(),
        // and killpg(). Treat that race as successful cancellation.
        Ok(()) | Err(Errno::ESRCH) => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn with_ignored_sigttou<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    // POSIX allows TOSTOP/SIGTTOU to stop a background process that changes
    // terminal settings. During cancellation the child may still be foreground,
    // so ignore SIGTTOU only around the foreground-group syscall and restore the
    // previous disposition immediately afterward.
    let _guard = SignalActionGuard::ignore(Signal::SIGTTOU)?;
    f()
}

struct SignalActionGuard {
    signal: Signal,
    previous: SigAction,
}

impl SignalActionGuard {
    fn ignore(signal: Signal) -> Result<Self> {
        let action = SigAction::new(SigHandler::SigIgn, SaFlags::empty(), SigSet::empty());
        // SAFETY: sigaction is called single-threaded (only from the main async task);
        // the previous disposition is saved and restored in Drop.
        let previous = unsafe { signal::sigaction(signal, &action) }?;
        Ok(Self { signal, previous })
    }
}

impl Drop for SignalActionGuard {
    fn drop(&mut self) {
        // SAFETY: restores the disposition saved by the matching sigaction in ignore().
        unsafe {
            signal::sigaction(self.signal, &self.previous).ok();
        }
    }
}

async fn wait_for_cancel(cancel: &mut watch::Receiver<u64>, version: u64) {
    while cancel.changed().await.is_ok() {
        if *cancel.borrow() != version {
            break;
        }
    }
}

fn wait_for_key() -> Result<()> {
    print!("<press key to run>");
    io::stdout().flush()?;
    let mut buf = [0_u8; 1];
    io::stdin().read_exact(&mut buf)?;
    println!();
    Ok(())
}
