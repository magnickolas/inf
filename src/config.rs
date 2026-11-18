use std::{
    collections::HashSet,
    env,
    ffi::OsStr,
    io::{self, IsTerminal, Read},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use anyhow::Result;

use crate::{cli::Cli, output::Colors};

#[derive(Clone, Debug)]
pub(crate) struct Config {
    pub(crate) compile_cmd: Option<String>,
    pub(crate) run_cmd: Option<String>,
    pub(crate) input_file: Option<PathBuf>,
    pub(crate) monitor_files: Vec<PathBuf>,
    pub(crate) noparse: bool,
    pub(crate) refresh: bool,
    pub(crate) postpone: bool,
    pub(crate) quiet: bool,
    pub(crate) waitkey: bool,
    pub(crate) zen: bool,
    pub(crate) verbose: bool,
    pub(crate) debug: bool,
    pub(crate) clear_screen: bool,
}

impl Config {
    pub(crate) fn from_cli(cli: Cli) -> Result<Self> {
        // The CLI parser keeps the compile command as trailing argv, but the
        // runtime intentionally stores a shell command string. The Bash version
        // treated the tail as one shell command, so preserving spaces here is
        // part of command-language compatibility rather than mere formatting.
        let compile_cmd = if cli.compile_cmd.is_empty() {
            None
        } else {
            Some(cli.compile_cmd.join(" "))
        };

        let mut config = Self {
            compile_cmd,
            run_cmd: cli.run_cmd,
            input_file: cli.input_file,
            monitor_files: Vec::new(),
            noparse: cli.noparse,
            refresh: cli.refresh,
            postpone: cli.postpone,
            quiet: cli.quiet,
            waitkey: cli.waitkey,
            zen: cli.zen,
            verbose: cli.verbose,
            debug: cli.debug,
            clear_screen: io::stdout().is_terminal() && env::var_os("NO_REFRESH").is_none(),
        };

        for monitor in cli.monitor {
            config.add_monitor_files(&monitor);
        }

        if let Some(input_file) = &config.input_file
            && config.run_cmd.is_some()
        {
            config.monitor_files.push(input_file.clone());
        }

        if !config.noparse && config.compile_cmd.is_some() {
            config.add_compile_files();
        }

        Ok(config)
    }

    pub(crate) fn add_stdin_monitor_files(&mut self) -> Result<()> {
        if io::stdin().is_terminal() {
            return Ok(());
        }

        let mut input = Vec::new();
        io::stdin().read_to_end(&mut input)?;
        if let Ok(text) = std::str::from_utf8(&input) {
            for token in text.split_whitespace() {
                self.add_monitor_files(OsStr::new(token));
            }
        } else {
            for token in input
                .split(u8::is_ascii_whitespace)
                .filter(|token| !token.is_empty())
            {
                self.add_monitor_files(OsStr::from_bytes(token));
            }
        }
        Ok(())
    }

    pub(crate) fn validate(&self, colors: &Colors) -> Result<(), u8> {
        if self.quiet && self.verbose {
            return Err(error(colors, "--quiet and --verbose conflict"));
        }

        if self.input_file.is_some() && self.run_cmd.is_none() {
            return Err(error(colors, "--input requires --run"));
        }

        if self.monitor_files.is_empty() {
            return Err(error(colors, "no files to monitor"));
        }

        let mut missing = Vec::new();
        for file in &self.monitor_files {
            if !file.is_file() {
                missing.push(file);
            }
        }

        if !missing.is_empty() {
            let prefix = if missing.len() > 1 { "files" } else { "file" };
            let mut msg = format!("missing {prefix} ");
            for file in missing {
                msg.push('"');
                msg.push_str(&file.to_string_lossy());
                msg.push_str("\" ");
            }
            return Err(error(colors, &msg));
        }

        Ok(())
    }

    pub(crate) fn watch_paths(&self) -> Vec<PathBuf> {
        let mut seen = HashSet::new();
        let mut paths = Vec::new();
        for file in &self.monitor_files {
            let path = if file.is_absolute() {
                file.clone()
            } else {
                env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(file)
            };
            if seen.insert(path.clone()) {
                paths.push(path);
            }
        }
        paths
    }

    fn add_monitor_files(&mut self, value: &OsStr) {
        for file in value.as_bytes().split(|&byte| byte == b',') {
            let path = expand_tilde(file);
            if path.is_absolute() {
                self.monitor_files.push(path);
            } else {
                self.monitor_files.push(
                    env::current_dir()
                        .unwrap_or_else(|_| PathBuf::from("."))
                        .join(path),
                );
            }
        }
    }

    fn add_compile_files(&mut self) {
        let Some(compile_cmd) = &self.compile_cmd else {
            return;
        };

        // This mirrors the old convenience heuristic, not a shell parser. It is
        // deliberately conservative: obvious `name.ext` tokens become watched
        // files, while shell constructs, quoted paths with spaces, response
        // files, generated source lists, etc. must be supplied with `--monitor`.
        let mut args = compile_cmd.split_whitespace();
        while let Some(arg) = args.next() {
            if matches!(arg, "-o" | "-MF") {
                // A compiler output can already exist and change on every run.
                args.next();
                continue;
            }
            if arg.contains('.') && Path::new(arg).is_file() {
                self.monitor_files.push(PathBuf::from(arg));
            }
        }
    }
}

fn expand_tilde(path: &[u8]) -> PathBuf {
    if let Some(rest) = path.strip_prefix(b"~/")
        && let Some(home) = env::var_os("HOME")
    {
        return PathBuf::from(home).join(OsStr::from_bytes(rest));
    }
    PathBuf::from(OsStr::from_bytes(path))
}

fn error(colors: &Colors, message: &str) -> u8 {
    eprintln!("{}ERROR: {}{}", colors.red, message, colors.reset);
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_comma_monitor_values() {
        let cli = Cli {
            run_cmd: None,
            input_file: None,
            monitor: vec!["a.txt,b.txt".into()],
            noparse: false,
            refresh: false,
            postpone: false,
            quiet: false,
            waitkey: false,
            zen: false,
            verbose: false,
            debug: false,
            compile_cmd: Vec::new(),
        };

        let config = Config::from_cli(cli).unwrap();
        assert_eq!(config.monitor_files.len(), 2);
        assert!(config.monitor_files[0].ends_with("a.txt"));
        assert!(config.monitor_files[1].ends_with("b.txt"));
    }

    #[test]
    fn detects_compile_file_args() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("main.c");
        std::fs::write(&source, "").unwrap();

        let cli = Cli {
            run_cmd: None,
            input_file: None,
            monitor: Vec::new(),
            noparse: false,
            refresh: false,
            postpone: false,
            quiet: false,
            waitkey: false,
            zen: false,
            verbose: false,
            debug: false,
            compile_cmd: vec![
                "gcc".into(),
                "-Wall".into(),
                source.to_string_lossy().into(),
            ],
        };

        let config = Config::from_cli(cli).unwrap();
        assert!(
            config
                .monitor_files
                .iter()
                .any(|path| path == Path::new(&source))
        );
    }
}
