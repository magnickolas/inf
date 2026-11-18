use std::{env, io::IsTerminal};

use crate::config::Config;

#[derive(Clone, Debug)]
pub(crate) struct Colors {
    pub(crate) blue: &'static str,
    pub(crate) green: &'static str,
    pub(crate) red: &'static str,
    pub(crate) cyan: &'static str,
    pub(crate) reset: &'static str,
}

impl Colors {
    pub(crate) fn detect() -> Self {
        if std::io::stdout().is_terminal() && env::var_os("NO_COLOR").is_none() {
            Self {
                blue: "\x1b[1;34m",
                green: "\x1b[1;32m",
                red: "\x1b[1;31m",
                cyan: "\x1b[0;34m",
                reset: "\x1b[0m",
            }
        } else {
            Self {
                blue: "",
                green: "",
                red: "",
                cyan: "",
                reset: "",
            }
        }
    }
}

pub(crate) fn debug_print(config: &Config, colors: &Colors) -> std::io::Result<()> {
    use std::io::{self, Write};

    if !config.debug {
        return Ok(());
    }

    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{}Parsed arguments:{}", colors.green, colors.reset)?;
    if !config.compile_cmd.is_empty() {
        let value = config.compile_cmd.join(std::ffi::OsStr::new(" "));
        writeln!(
            stdout,
            "{}compileCmd{}={}",
            colors.cyan,
            colors.reset,
            value.to_string_lossy()
        )?;
    }
    if let Some(value) = &config.run_cmd {
        let rendered = if let Some(input) = &config.input_file {
            format!("{value} <{}", input.display())
        } else {
            value.clone()
        };
        writeln!(stdout, "{}runCmd{}={rendered}", colors.cyan, colors.reset)?;
    }
    if let Some(value) = &config.input_file {
        writeln!(
            stdout,
            "{}inputfile{}={}",
            colors.cyan,
            colors.reset,
            value.display()
        )?;
    }

    for (name, value) in [
        ("noparse", config.noparse),
        ("refresh", config.refresh),
        ("compile", !config.compile_cmd.is_empty()),
        ("run", config.run_cmd.is_some()),
        ("input", config.input_file.is_some()),
        ("verbose", config.verbose),
        ("postpone", config.postpone),
        ("quiet", config.quiet),
        ("waitkey", config.waitkey),
        ("zen", config.zen),
        ("debug", config.debug),
    ] {
        writeln!(
            stdout,
            "{}{}{}={}",
            colors.cyan,
            name,
            colors.reset,
            if value { 1 } else { 0 }
        )?;
    }

    let files = config
        .monitor_files
        .iter()
        .map(|path| path.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    writeln!(
        stdout,
        "{}monitorFiles[*]{}={files}",
        colors.cyan, colors.reset
    )?;
    stdout.flush()
}
