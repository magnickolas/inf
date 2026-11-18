use std::{
    ffi::OsStr,
    fs::{File, OpenOptions},
    io::{self, IsTerminal},
    os::{
        fd::{AsFd, AsRawFd, RawFd},
        unix::{fs::OpenOptionsExt, process::ExitStatusExt},
    },
    path::Path,
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
    io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    process::{ChildStderr, ChildStdout, Command},
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

    let mut banner = String::new();
    if config.clear_screen {
        banner.push_str("\x1b[2J\x1b[H");
    }
    if !config.compile_cmd.is_empty() && !config.zen {
        banner.push_str(&format!(
            "{}[compilation: {}]{}\n",
            colors.blue,
            config.compile_cmd.join(OsStr::new(" ")).to_string_lossy(),
            colors.reset
        ));
    }
    if !write_run_output(libc::STDOUT_FILENO, &banner, &mut cancel).await? {
        return Ok(RunResult::Cancelled);
    }

    if config.run_cmd.is_some() {
        if !config.compile_cmd.is_empty() {
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
    } else if !config.compile_cmd.is_empty() {
        match run_compile(&config, &mut cancel).await? {
            CommandResult::Cancelled => Ok(RunResult::Cancelled),
            CommandResult::Finished(status) => {
                if status.success()
                    && !config.zen
                    && !write_run_output(
                        libc::STDOUT_FILENO,
                        &format!("{}Compilation succeeded!{}\n", colors.green, colors.reset),
                        &mut cancel,
                    )
                    .await?
                {
                    return Ok(RunResult::Cancelled);
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
    let mut cmd = Command::new("bash");
    match config.compile_cmd.as_slice() {
        [script] => cmd.arg("-c").arg(script),
        args => cmd.arg("-c").arg(r#""$@""#).arg("inf").args(args),
    };

    let mode = if config.verbose {
        OutputMode::Inherit
    } else if config.quiet {
        OutputMode::Suppress
    } else {
        OutputMode::CaptureStdout
    };
    run_command(cmd, None, mode, cancel).await
}

async fn run_target(
    config: &Config,
    colors: &Colors,
    cancel: &mut watch::Receiver<u64>,
) -> Result<RunResult> {
    let command = config.run_cmd.as_deref().unwrap_or_default();
    if !config.zen
        && !write_run_output(
            libc::STDOUT_FILENO,
            &format!(
                "{}{}{}\n",
                colors.green,
                command.to_string_lossy(),
                colors.reset
            ),
            cancel,
        )
        .await?
    {
        return Ok(RunResult::Cancelled);
    }

    let input = if let Some(path) = &config.input_file {
        match open_run_input(path) {
            Ok(file) => Some(file),
            Err(err) => {
                let written = write_run_output(
                    libc::STDERR_FILENO,
                    &format!("ERROR: failed to open input file {path:?}: {err}\n"),
                    cancel,
                )
                .await?;
                return Ok(if written {
                    RunResult::Completed
                } else {
                    RunResult::Cancelled
                });
            }
        }
    } else {
        None
    };
    let CommandResult::Finished(status) =
        run_shell(command, input, OutputMode::Inherit, cancel).await?
    else {
        return Ok(RunResult::Cancelled);
    };
    let message = if let Some(number) = status.signal() {
        let signal = Signal::try_from(number)
            .map_or_else(|_| format!("signal {number}"), |signal| signal.to_string());
        Some(format!(
            "{}[terminated by {signal}]{}\n",
            colors.red, colors.reset
        ))
    } else if let Some(code) = status.code().filter(|&code| code != 0) {
        Some(format!(
            "{}[exit code = {code}]{}\n",
            colors.red, colors.reset
        ))
    } else if !config.zen {
        Some(format!(
            "{}[execution succeeded]{}\n",
            colors.green, colors.reset
        ))
    } else {
        None
    };
    if let Some(message) = message
        && !write_run_output(libc::STDOUT_FILENO, &message, cancel).await?
    {
        return Ok(RunResult::Cancelled);
    }
    Ok(RunResult::Completed)
}

fn open_run_input(path: &Path) -> io::Result<File> {
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
    command: &OsStr,
    input: Option<File>,
    mode: OutputMode,
    cancel: &mut watch::Receiver<u64>,
) -> Result<CommandResult> {
    let mut cmd = Command::new("bash");
    cmd.arg("-c").arg(command);
    run_command(cmd, input, mode, cancel).await
}

async fn run_command(
    mut cmd: Command,
    input: Option<File>,
    mode: OutputMode,
    cancel: &mut watch::Receiver<u64>,
) -> Result<CommandResult> {
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

    match mode {
        OutputMode::Inherit => run_in_foreground(cmd, cancel).await,
        OutputMode::CaptureStdout | OutputMode::Suppress => run_captured(cmd, mode, cancel).await,
    }
}

async fn run_in_foreground(
    cmd: Command,
    cancel: &mut watch::Receiver<u64>,
) -> Result<CommandResult> {
    let mut child = spawn_foreground(cmd)?;
    tokio::select! {
        status = child.wait() => Ok(CommandResult::Finished(status?)),
        _ = wait_for_cancel(cancel) => {
            child.restore_terminal();
            child.terminate_gracefully().await?;
            Ok(CommandResult::Cancelled)
        }
    }
}

async fn run_captured(
    mut cmd: Command,
    mode: OutputMode,
    cancel: &mut watch::Receiver<u64>,
) -> Result<CommandResult> {
    let capture = if matches!(mode, OutputMode::CaptureStdout) {
        match capture_file_limit().and_then(|limit| {
            tempfile::tempfile()
                .or_else(|_| tempfile::tempfile_in("/tmp"))
                .map(|file| (fs::File::from_std(file), limit))
        }) {
            Ok(capture) => Some(capture),
            Err(err) => {
                if !write_run_output(
                    libc::STDERR_FILENO,
                    &format!("Failed to capture compiler output, streaming it instead: {err}\n"),
                    cancel,
                )
                .await?
                {
                    return Ok(CommandResult::Cancelled);
                }
                cmd.stdout(Stdio::inherit());
                None
            }
        }
    } else {
        None
    };

    let mut child = CapturedChild::spawn(cmd)?;
    let (mut stdout, mut stderr) = child.take_output()?;
    let mut stdout_task = tokio::spawn(async move {
        let Some(out) = stdout.as_mut() else {
            return Ok(None);
        };
        if let Some((file, limit)) = capture {
            return capture_compiler_stdout(out, file, limit).await.map(Some);
        }
        tokio::io::copy(out, &mut tokio::io::sink()).await?;
        Ok(None)
    });
    let mut stderr_task = tokio::spawn(async move {
        if let Some(err) = stderr.as_mut() {
            tokio::io::copy(err, &mut tokio::io::sink()).await?;
        }
        Ok::<_, io::Error>(())
    });

    let (status, capture) = tokio::select! {
        result = async {
            let status = child.wait().await?;
            let capture = (&mut stdout_task).await??;
            (&mut stderr_task).await??;
            Ok::<_, anyhow::Error>((status, capture))
        } => result?,
        _ = wait_for_cancel(cancel) => {
            child.kill().await;
            stdout_task.abort();
            stderr_task.abort();
            return Ok(CommandResult::Cancelled);
        }
    };
    drop(child);
    match capture {
        Some(Ok(file)) if !status.success() => {
            tokio::select! {
                result = replay_compiler_output(file) => result?,
                _ = wait_for_cancel(cancel) => return Ok(CommandResult::Cancelled),
            }
        }
        Some(Err(err)) => {
            let written = write_run_output(
                libc::STDERR_FILENO,
                &format!("Failed to save compiler stdout: {err}\n"),
                cancel,
            )
            .await?;
            if !written {
                return Ok(CommandResult::Cancelled);
            }
        }
        _ => {}
    }
    Ok(CommandResult::Finished(status))
}

fn capture_file_limit() -> io::Result<u64> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: limit points to a writable rlimit for this syscall.
    if unsafe { libc::getrlimit(libc::RLIMIT_FSIZE, &mut limit) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(if limit.rlim_cur == libc::RLIM_INFINITY {
        u64::MAX
    } else {
        limit.rlim_cur as _
    })
}

async fn capture_compiler_stdout(
    stdout: &mut (impl AsyncRead + Unpin),
    mut file: fs::File,
    limit: u64,
) -> io::Result<io::Result<fs::File>> {
    let saved = async {
        // Keep spool writes below RLIMIT_FSIZE to avoid SIGXFSZ.
        tokio::io::copy(&mut stdout.take(limit), &mut file).await?;
        file.flush().await
    }
    .await;
    // Keep draining the pipe so a spool failure cannot stop the compiler.
    let discarded = tokio::io::copy(stdout, &mut tokio::io::sink()).await?;
    Ok(saved.and_then(|()| {
        if discarded == 0 {
            Ok(file)
        } else {
            Err(io::Error::from_raw_os_error(libc::EFBIG))
        }
    }))
}

async fn replay_compiler_output(mut file: fs::File) -> io::Result<()> {
    file.rewind().await?;
    let mut buffer = [0; 8192];
    loop {
        let len = file.read(&mut buffer).await?;
        if len == 0 {
            return Ok(());
        }
        write_output(libc::STDOUT_FILENO, &buffer[..len]).await?;
    }
}

async fn write_run_output(
    fd: RawFd,
    message: &str,
    cancel: &mut watch::Receiver<u64>,
) -> io::Result<bool> {
    tokio::select! {
        biased;
        _ = wait_for_cancel(cancel) => Ok(false),
        result = write_output(fd, message.as_bytes()) => result.map(|()| true),
    }
}

async fn write_output(fd: RawFd, mut remaining: &[u8]) -> io::Result<()> {
    while !remaining.is_empty() {
        match try_write_output(fd, remaining) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(written) => remaining = &remaining[written..],
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                time::sleep(CHILD_POLL_INTERVAL).await;
            }
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

fn try_write_output(fd: RawFd, bytes: &[u8]) -> io::Result<usize> {
    // SAFETY: The slice stays valid for the duration of the write. Restore
    // the descriptor flags before yielding because other processes may share them.
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
        status: None,
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
    status: Option<ExitStatus>,
    original_pgid: Option<Pid>,
    original_termios: Option<Termios>,
}

impl ForegroundChild {
    async fn wait(&mut self) -> Result<ExitStatus> {
        let status = loop {
            match waitpid(
                Some(self.pgid),
                Some(WaitPidFlag::WNOHANG | WaitPidFlag::WUNTRACED),
            ) {
                Ok(WaitStatus::Exited(_, code)) => break ExitStatus::from_raw(code << 8),
                Ok(WaitStatus::Signaled(_, signal, core_dumped)) => {
                    let raw = signal as i32 | if core_dumped { 0x80 } else { 0 };
                    break ExitStatus::from_raw(raw);
                }
                Ok(WaitStatus::Stopped(_, signal)) => {
                    self.restore_terminal();
                    let status = self.kill().await?;
                    self.status = Some(status);
                    write_output(
                        libc::STDERR_FILENO,
                        format!("Command stopped by {signal}, ending this run\n").as_bytes(),
                    )
                    .await?;
                    break status;
                }
                Ok(_) | Err(Errno::EINTR) => {}
                Err(err) => return Err(err).context("failed to wait for foreground command"),
            }
            time::sleep(CHILD_POLL_INTERVAL).await;
        };
        self.status = Some(status);
        Ok(status)
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

    async fn kill(&mut self) -> Result<ExitStatus> {
        ignore_no_process(killpg(self.pgid, Signal::SIGKILL))?;
        if let Some(status) = self.status {
            return Ok(status);
        }
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

enum CapturedChild {
    Foreground(ForegroundChild),
    Background(AsyncGroupChild),
}

impl CapturedChild {
    fn spawn(mut cmd: Command) -> Result<Self> {
        if io::stdin().is_terminal() && original_foreground_pgid() == Some(getpgrp()) {
            return spawn_foreground(cmd).map(Self::Foreground);
        }

        let child = cmd
            .group()
            .kill_on_drop(true)
            .spawn()
            .context("failed to spawn command")?;
        Ok(Self::Background(child))
    }

    fn take_output(&mut self) -> io::Result<(Option<ChildStdout>, Option<ChildStderr>)> {
        match self {
            Self::Foreground(child) => Ok((
                child
                    .child
                    .stdout
                    .take()
                    .map(ChildStdout::from_std)
                    .transpose()?,
                child
                    .child
                    .stderr
                    .take()
                    .map(ChildStderr::from_std)
                    .transpose()?,
            )),
            Self::Background(child) => {
                let child = child.inner();
                Ok((child.stdout.take(), child.stderr.take()))
            }
        }
    }

    async fn wait(&mut self) -> Result<ExitStatus> {
        match self {
            Self::Foreground(child) => child.wait().await,
            Self::Background(child) => Ok(child.wait().await?),
        }
    }

    fn restore_terminal(&mut self) {
        if let Self::Foreground(child) = self {
            child.restore_terminal();
        }
    }

    async fn kill(&mut self) {
        self.restore_terminal();
        match self {
            Self::Foreground(child) => {
                let _ = child.kill().await;
            }
            Self::Background(child) => {
                let _ = child.kill().await;
            }
        }
    }
}

impl Drop for CapturedChild {
    fn drop(&mut self) {
        if let Self::Background(child) = self {
            // The shell can exit and close its pipes while its group lives on.
            let _ = child.start_kill();
        }
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

async fn wait_for_cancel(cancel: &mut watch::Receiver<u64>) {
    // Each run starts at zero. Keep cancellation visible across all stages.
    let _ = cancel.wait_for(|&version| version != 0).await;
}

async fn wait_for_key(cancel: &mut watch::Receiver<u64>) -> Result<bool> {
    let stdin = io::stdin();
    let terminal = if stdin.is_terminal() {
        None
    } else {
        Some(File::open("/dev/tty").context("--waitkey requires a terminal")?)
    };
    let input = terminal.as_ref().map_or_else(|| stdin.as_fd(), File::as_fd);

    if !write_run_output(libc::STDOUT_FILENO, "<press Enter to run>", cancel).await? {
        return Ok(false);
    }
    let mut poll_fd = libc::pollfd {
        fd: input.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        // SAFETY: poll_fd describes the open input descriptor and the timeout is zero.
        let ready = unsafe { libc::poll(&mut poll_fd, 1, 0) };
        if ready > 0 {
            let mut buf = [0_u8; 1];
            if read(input, &mut buf)? == 0 {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
            }
            if buf[0] == b'\n' {
                return Ok(write_run_output(libc::STDOUT_FILENO, "\n", cancel).await?);
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
            _ = wait_for_cancel(cancel) => return Ok(false),
            _ = time::sleep(CHILD_POLL_INTERVAL) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn capture_keeps_draining_after_a_spool_write_error() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let read_only = fs::File::open(temp.path()).await.unwrap();
        let mut output = &[b'a'; 32 * 1024][..];
        let capture = capture_compiler_stdout(&mut output, read_only, u64::MAX)
            .await
            .unwrap();

        assert!(capture.is_err());
        assert!(output.is_empty());
    }
}
