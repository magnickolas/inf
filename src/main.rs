#[cfg(not(unix))]
compile_error!("inf currently supports Unix-like platforms only");

mod cli;
mod command;
mod config;
mod output;
mod watch;

use std::process::ExitCode;

use anyhow::Result;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            eprintln!("ERROR: {err}");
            ExitCode::from(1)
        }
    }
}

async fn run() -> Result<u8> {
    let colors = output::Colors::detect();
    let cli = match cli::parse(std::env::args().collect())? {
        cli::ParseOutcome::Exit { code } => return Ok(code),
        cli::ParseOutcome::Run(cli) => cli,
    };

    let mut config = config::Config::from_cli(cli)?;
    config.add_stdin_monitor_files()?;
    output::debug_print(&config, &colors);
    if let Err(code) = config.validate(&colors) {
        return Ok(code);
    }

    if config.debug {
        return Ok(0);
    }

    watch::run(config, colors).await?;
    Ok(0)
}
