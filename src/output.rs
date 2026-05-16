use std::{env, io::IsTerminal};

use crate::config::Config;

#[derive(Clone, Debug)]
pub struct Colors {
    pub blue: &'static str,
    pub green: &'static str,
    pub red: &'static str,
    pub cyan: &'static str,
    pub reset: &'static str,
}

impl Colors {
    pub fn detect() -> Self {
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

pub fn debug_print(config: &Config, colors: &Colors) {
    if !config.debug {
        return;
    }

    println!("{}Parsed arguments:{}", colors.green, colors.reset);
    if let Some(value) = &config.compile_cmd {
        println!("{}compileCmd{}={value}", colors.cyan, colors.reset);
    }
    if let Some(value) = &config.run_cmd {
        let rendered = if let Some(input) = &config.input_file {
            format!("{value} <{input}")
        } else {
            value.clone()
        };
        println!("{}runCmd{}={rendered}", colors.cyan, colors.reset);
    }
    if let Some(value) = &config.input_file {
        println!("{}inputfile{}={value}", colors.cyan, colors.reset);
    }

    for (name, value) in [
        ("noparse", config.noparse),
        ("refresh", config.refresh),
        ("compile", config.compile_cmd.is_some()),
        ("run", config.run_cmd.is_some()),
        ("input", config.input_file.is_some()),
        ("verbose", config.verbose),
        ("postpone", config.postpone),
        ("quiet", config.quiet),
        ("waitkey", config.waitkey),
        ("zen", config.zen),
        ("debug", config.debug),
    ] {
        println!(
            "{}{}{}={}",
            colors.cyan,
            name,
            colors.reset,
            if value { 1 } else { 0 }
        );
    }

    let files = config
        .monitor_files
        .iter()
        .map(|path| path.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    println!("{}monitorFiles[*]{}={files}", colors.cyan, colors.reset);
}
