use std::{
    collections::HashSet,
    env,
    ffi::{OsStr, OsString},
    io::{self, IsTerminal, Read, Write},
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};

use anyhow::Result;

use crate::{cli::Cli, output::Colors};

#[derive(Clone, Debug)]
pub(crate) struct Config {
    pub(crate) compile_cmd: Vec<OsString>,
    pub(crate) run_cmd: Option<OsString>,
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
        let mut config = Self {
            compile_cmd: cli.compile_cmd,
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

        if !config.noparse && !config.compile_cmd.is_empty() {
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
        if input.contains(&0) {
            for path in input
                .split(|&byte| byte == 0)
                .filter(|path| !path.is_empty())
            {
                self.add_monitor_path(PathBuf::from(OsStr::from_bytes(path)));
            }
        } else {
            for token in split_words(OsStr::from_bytes(&input)) {
                self.add_monitor_files(token);
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
        let mut bytes = value.as_bytes().iter().copied().peekable();
        let mut path = Vec::new();
        while let Some(byte) = bytes.next() {
            match byte {
                b',' => {
                    self.add_monitor_path(expand_tilde(&path));
                    path.clear();
                }
                b'\\' if matches!(bytes.peek(), Some(b',' | b'\\')) => {
                    path.extend(bytes.next());
                }
                _ => path.push(byte),
            }
        }
        self.add_monitor_path(expand_tilde(&path));
    }

    fn add_monitor_path(&mut self, path: PathBuf) {
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

    fn add_compile_files(&mut self) {
        // Split a script into shell words for autodetection. Bash still handles
        // expansions and execution when the actual command runs.
        let shell_words: Vec<OsString>;
        let mut args = if let [script] = self.compile_cmd.as_slice() {
            shell_words = match shlex::bytes::split(script.as_bytes()) {
                Some(words) => words.into_iter().map(OsString::from_vec).collect(),
                None => split_words(script)
                    .into_iter()
                    .map(OsStr::to_owned)
                    .collect(),
            };
            shell_words.iter()
        } else {
            self.compile_cmd.iter()
        };
        while let Some(arg) = args.next() {
            if matches!(
                arg.as_bytes(),
                b"-o" | b"-MF" | b"-MJ" | b"-serialize-diagnostics" | b"--serialize-diagnostics"
            ) {
                // A compiler output can already exist and change on every run.
                args.next();
                continue;
            }
            if arg.as_bytes().contains(&b'.') && Path::new(arg).is_file() {
                self.monitor_files.push(PathBuf::from(arg));
            }
        }
    }
}

fn split_words(value: &OsStr) -> Vec<&OsStr> {
    if let Some(text) = value.to_str() {
        text.split_whitespace().map(OsStr::new).collect()
    } else {
        value
            .as_bytes()
            .split(u8::is_ascii_whitespace)
            .filter(|token| !token.is_empty())
            .map(OsStr::from_bytes)
            .collect()
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
    let _ = writeln!(
        io::stderr().lock(),
        "{}ERROR: {}{}",
        colors.red,
        message,
        colors.reset
    );
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
    fn detects_compile_inputs_and_skips_clang_outputs() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("main.c");
        std::fs::write(&source, "").unwrap();
        let database = temp.path().join("compile.json");
        let diagnostics = temp.path().join("diagnostics.dia");
        let diagnostics_alias = temp.path().join("alias.dia");
        for output in [&database, &diagnostics, &diagnostics_alias] {
            std::fs::write(output, "").unwrap();
        }

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
                "clang".into(),
                "-MJ".into(),
                database.as_os_str().into(),
                "--serialize-diagnostics".into(),
                diagnostics.as_os_str().into(),
                "-serialize-diagnostics".into(),
                diagnostics_alias.as_os_str().into(),
                source.as_os_str().into(),
            ],
        };

        let config = Config::from_cli(cli).unwrap();
        assert_eq!(config.monitor_files, [source]);
    }
}
