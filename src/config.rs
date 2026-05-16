use std::{
    collections::HashSet,
    env,
    io::{self, IsTerminal, Read},
    path::{Path, PathBuf},
};

use anyhow::Result;

use crate::{cli::Cli, output::Colors};

#[derive(Clone, Debug)]
pub struct Config {
    pub compile_cmd: Option<String>,
    pub run_cmd: Option<String>,
    pub input_file: Option<String>,
    pub monitor_files: Vec<PathBuf>,
    pub noparse: bool,
    pub refresh: bool,
    pub postpone: bool,
    pub quiet: bool,
    pub waitkey: bool,
    pub zen: bool,
    pub verbose: bool,
    pub debug: bool,
    pub clear_screen: bool,
}

impl Config {
    pub fn from_cli(cli: Cli) -> Result<Self> {
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
            config.monitor_files.push(PathBuf::from(input_file));
        }

        if !config.noparse && config.compile_cmd.is_some() {
            config.add_compile_files();
        }

        Ok(config)
    }

    pub fn add_stdin_monitor_files(&mut self) -> Result<()> {
        if io::stdin().is_terminal() {
            return Ok(());
        }

        let mut input = String::new();
        io::stdin().read_to_string(&mut input)?;
        for token in input.split_whitespace() {
            self.add_monitor_files(token);
        }
        Ok(())
    }

    pub fn validate(&self, colors: &Colors) -> Result<(), u8> {
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

    pub fn watch_paths(&self) -> Vec<PathBuf> {
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

    fn add_monitor_files(&mut self, value: &str) {
        for file in value.split(',') {
            let expanded = expand_tilde(file);
            let path = PathBuf::from(&expanded);
            if path.exists() {
                // Existing explicit monitor paths are canonicalized to collapse
                // aliases and keep watchexec from seeing duplicate spellings of
                // the same file. Missing paths are left as user-facing absolute
                // paths so validation can report the exact missing file.
                match path.canonicalize() {
                    Ok(path) => self.monitor_files.push(path),
                    Err(_) => self.monitor_files.push(path),
                }
            } else if path.is_absolute() {
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
        for arg in compile_cmd.split_whitespace() {
            if looks_like_file_arg(arg) && Path::new(arg).is_file() {
                self.monitor_files.push(PathBuf::from(arg));
            }
        }
    }
}

fn looks_like_file_arg(arg: &str) -> bool {
    !arg.chars().any(char::is_whitespace) && arg.contains('.') && !arg.starts_with('.')
}

fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = env::var_os("HOME")
    {
        return PathBuf::from(home)
            .join(rest)
            .to_string_lossy()
            .into_owned();
    }
    path.to_string()
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
            monitor: vec!["a.txt,b.txt".to_string()],
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
