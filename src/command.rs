use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    os::unix::{fs::OpenOptionsExt, process::ExitStatusExt},
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
        wait::{WaitPidFlag, WaitStatus, waitpid},
    },
    unistd::{Pid, getpgrp, read, tcgetpgrp, tcsetpgrp},
};
use tokio::sync::watch;
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    process::Command,
    time,
};

use crate::{config::Config, output::Colors};

const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(20);
const TERMINATE_GRACE_PERIOD: Duration = Duration::from_millis(500);

#[derive(Debug)]
pub(crate) enum RunResult {
    Completed,
    Cancelled,
}

pub(crate) async fn compile_and_run(
    config: Arc<Config>,
    colors: Arc<Colors>,
    mut cancel: watch::Receiver<u64>,
) -> Result<RunResult> {
    if config.waitkey && !wait_for_key(&mut cancel).await? {
        return Ok(RunResult::Cancelled);
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

        run_target(&config, &colors, &mut cancel).await
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
        OutputMode::Suppress
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
) -> Result<RunResult> {
    let command = config.run_cmd.as_deref().unwrap_or("");
    if !config.zen {
        println!("{}{}{}", colors.green, command, colors.reset);
    }

    let input = if let Some(path) = &config.input_file {
        match open_run_input(path) {
            Ok(file) => Some(file),
            Err(err) => {
                eprintln!("ERROR: failed to open input file {path:?}: {err}");
                return Ok(RunResult::Completed);
            }
        }
    } else {
        None
    };
    let result = run_shell(command, input, OutputMode::Inherit, cancel).await?;
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
    Ok(match result {
        CommandResult::Cancelled => RunResult::Cancelled,
        CommandResult::Finished(_) => RunResult::Completed,
    })
}

fn open_run_input(path: &str) -> io::Result<File> {
    // Validate the opened file so a replacement between validation and open
    // cannot leave us blocked waiting for a FIFO writer.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a regular file",
        ));
    }
    Ok(file)
}

#[derive(Clone, Copy)]
enum OutputMode {
    Inherit,
    CaptureStdout,
    Suppress,
}

async fn run_shell(
    command: &str,
    input: Option<File>,
    mode: OutputMode,
    cancel: &mut watch::Receiver<u64>,
) -> Result<CommandResult> {
    match mode {
        OutputMode::Inherit => {
            let cmd = shell_command(command, input, mode);
            run_in_foreground(cmd, cancel).await
        }
        OutputMode::CaptureStdout | OutputMode::Suppress => {
            let cmd = shell_command(command, input, mode);
            run_captured(cmd, mode, cancel).await
        }
    }
}

fn shell_command(command: &str, input: Option<File>, mode: OutputMode) -> Command {
    let mut cmd = Command::new("bash");
    cmd.arg("-c").arg(command);

    match mode {
        OutputMode::Inherit => {
            cmd.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        }
        OutputMode::CaptureStdout => {
            cmd.stdout(Stdio::piped()).stderr(Stdio::inherit());
        }
        OutputMode::Suppress => {
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        }
    }

    if let Some(file) = input {
        cmd.stdin(Stdio::from(file));
    }

    cmd
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
    mut cmd: Command,
    mode: OutputMode,
    cancel: &mut watch::Receiver<u64>,
) -> Result<CommandResult> {
    let version = *cancel.borrow();
    // Captured commands are not interactive, so they do not need terminal
    // foreground ownership. They still run in a process group so cancellation
    // can stop descendant processes.
    let mut capture = if matches!(mode, OutputMode::CaptureStdout) {
        match tempfile::tempfile().or_else(|_| tempfile::tempfile_in("/tmp")) {
            Ok(file) => Some(fs::File::from_std(file)),
            Err(err) => {
                eprintln!("Failed to capture compiler output, streaming it instead: {err}");
                cmd.stdout(Stdio::inherit());
                None
            }
        }
    } else {
        None
    };

    let mut group = ChildGroup::spawn(cmd)?;
    let mut stdout = group.inner_mut().stdout.take();
    let mut stderr = group.inner_mut().stderr.take();
    let mut stdout_task = tokio::spawn(async move {
        if let Some(out) = stdout.as_mut() {
            if let Some(file) = capture.as_mut() {
                tokio::io::copy(out, file).await?;
                file.flush().await?;
            } else {
                tokio::io::copy(out, &mut tokio::io::sink()).await?;
            }
        }
        Ok::<_, io::Error>(capture)
    });
    let mut stderr_task = tokio::spawn(async move {
        if let Some(err) = stderr.as_mut() {
            tokio::io::copy(err, &mut tokio::io::sink()).await?;
        }
        Ok::<_, io::Error>(())
    });

    tokio::select! {
        result = async {
            let status = group.wait().await?;
            let capture = (&mut stdout_task).await??;
            (&mut stderr_task).await??;
            if !status.success() && let Some(file) = capture {
                replay_compiler_output(file).await?;
            }
            Ok(CommandResult::Finished(status))
        } => result,
        _ = wait_for_cancel(cancel, version) => {
            group.kill().await;
            stdout_task.abort();
            stderr_task.abort();
            Ok(CommandResult::Cancelled)
        }
    }
}

async fn replay_compiler_output(mut file: fs::File) -> io::Result<()> {
    file.rewind().await?;
    let mut buffer = [0; 8192];
    loop {
        let len = file.read(&mut buffer).await?;
        if len == 0 {
            return Ok(());
        }
        let mut remaining = &buffer[..len];
        while !remaining.is_empty() {
            match try_write_stdout(remaining) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(written) => remaining = &remaining[written..],
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    time::sleep(CHILD_POLL_INTERVAL).await;
                }
                Err(err) => return Err(err),
            }
        }
    }
}

fn try_write_stdout(bytes: &[u8]) -> io::Result<usize> {
    let fd = libc::STDOUT_FILENO;
    // SAFETY: The slice stays valid for the duration of the write. Restore
    // stdout's flags before yielding because other processes may share them.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags == -1 {
            return Err(io::Error::last_os_error());
        }
        let restore = flags & libc::O_NONBLOCK == 0;
        if restore && libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) == -1 {
            return Err(io::Error::last_os_error());
        }
        let written = libc::write(fd, bytes.as_ptr().cast(), bytes.len());
        let result = if written == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(written as usize)
        };
        if restore && libc::fcntl(fd, libc::F_SETFL, flags) == -1 {
            return Err(io::Error::last_os_error());
        }
        result
    }
}

async fn wait_for_foreground_child(
    child: &mut ForegroundChild,
    cancel: &mut watch::Receiver<u64>,
    version: u64,
) -> Result<CommandResult> {
    loop {
        match waitpid(
            Some(child.pgid),
            Some(WaitPidFlag::WNOHANG | WaitPidFlag::WUNTRACED),
        ) {
            Ok(WaitStatus::Exited(_, code)) => {
                return Ok(CommandResult::Finished(ExitStatus::from_raw(code << 8)));
            }
            Ok(WaitStatus::Signaled(_, signal, core_dumped)) => {
                let status = signal as i32 | if core_dumped { 0x80 } else { 0 };
                return Ok(CommandResult::Finished(ExitStatus::from_raw(status)));
            }
            Ok(WaitStatus::Stopped(_, signal)) => {
                child.restore_terminal();
                eprintln!("Command stopped by {signal}, ending this run");
                let status = child.kill().await?;
                return Ok(CommandResult::Finished(status));
            }
            Ok(_) | Err(Errno::EINTR) => {}
            Err(err) => return Err(err).context("failed to wait for foreground command"),
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
    use std::os::unix::process::CommandExt;

    let cmd = cmd.as_std_mut();
    // The child must be in a separate process group so we can kill the whole
    // subtree, but a background group gets SIGTTIN on terminal reads. The child
    // claims terminal foreground itself in pre_exec — after setpgid but before
    // exec — so by the time bash starts there is no race window.
    let original_pgid = original_foreground_pgid().filter(|&pgid| pgid == getpgrp());
    let original_termios = original_pgid.and_then(|_| save_termios());

    if let Some(original) = original_pgid {
        // SAFETY: These syscalls are async-signal-safe between fork and exec.
        unsafe {
            cmd.pre_exec(move || {
                let tty_fd = libc::open(c"/dev/tty".as_ptr(), libc::O_RDWR);
                if tty_fd < 0 {
                    return Ok(());
                }
                if libc::tcgetpgrp(tty_fd) == original.as_raw() {
                    libc::signal(libc::SIGTTOU, libc::SIG_IGN);
                    libc::tcsetpgrp(tty_fd, libc::getpid());
                    libc::signal(libc::SIGTTOU, libc::SIG_DFL);
                }
                libc::close(tty_fd);
                Ok(())
            });
        }
    }

    cmd.process_group(0);
    let child = cmd.spawn().context("failed to spawn foreground command")?;

    let pgid = Pid::from_raw(child.id() as i32);
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
    child: std::process::Child,
    pgid: Pid,
    original_pgid: Option<Pid>,
    original_termios: Option<Termios>,
}

impl ForegroundChild {
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

    async fn kill(&mut self) -> Result<ExitStatus> {
        ignore_no_process(killpg(self.pgid, Signal::SIGKILL))?;
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            time::sleep(CHILD_POLL_INTERVAL).await;
        }
    }

    async fn terminate_gracefully(&mut self) -> Result<()> {
        ignore_no_process(killpg(self.pgid, Signal::SIGTERM))?;

        let deadline = time::sleep(TERMINATE_GRACE_PERIOD);
        tokio::pin!(deadline);
        loop {
            if self.child.try_wait()?.is_some() {
                // The leader can exit while descendants still need SIGKILL.
                match killpg(self.pgid, None) {
                    Err(Errno::ESRCH) => return Ok(()),
                    result => result?,
                }
            }
            tokio::select! {
                _ = &mut deadline => {
                    self.kill().await?;
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
        // The shell can exit before its background descendants.
        ignore_no_process(killpg(self.pgid, Signal::SIGKILL)).ok();
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

async fn wait_for_key(cancel: &mut watch::Receiver<u64>) -> Result<bool> {
    print!("<press Enter to run>");
    io::stdout().flush()?;
    let version = *cancel.borrow();
    let mut stdin = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        // SAFETY: stdin is a valid pollfd and the zero timeout never blocks.
        let ready = unsafe { libc::poll(&mut stdin, 1, 0) };
        if ready > 0 {
            let mut buf = [0_u8; 1];
            if read(io::stdin(), &mut buf)? == 0 {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
            }
            if buf[0] == b'\n' {
                println!();
                return Ok(true);
            }
            continue;
        }
        if ready < 0 {
            let err = io::Error::last_os_error();
            if err.kind() != io::ErrorKind::Interrupted {
                return Err(err.into());
            }
        }
        tokio::select! {
            _ = wait_for_cancel(cancel, version) => return Ok(false),
            _ = time::sleep(CHILD_POLL_INTERVAL) => {}
        }
    }
}
