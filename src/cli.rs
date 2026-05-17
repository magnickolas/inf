use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "inf",
    version,
    arg_required_else_help = true,
    override_usage = "inf [OPTIONS] [COMPILE_CMD]...",
    trailing_var_arg = true
)]
pub(crate) struct Cli {
    #[arg(
        short = 'r',
        long = "run",
        value_name = "RUN_CMD",
        help = "Target execution command"
    )]
    pub(crate) run_cmd: Option<String>,

    #[arg(
        short = 'i',
        long = "input",
        value_name = "INPUT_FILE",
        help = "Input file"
    )]
    pub(crate) input_file: Option<String>,

    #[arg(
        short = 'm',
        long = "monitor",
        value_name = "FILE[,FILE...]",
        help = "Comma separated list of files to trigger recompilation"
    )]
    pub(crate) monitor: Vec<String>,

    #[arg(
        short = 'n',
        long = "noparse",
        help = "Don't look for *.* names patterns in compile command"
    )]
    pub(crate) noparse: bool,

    #[arg(
        short = 'x',
        long = "refresh",
        help = "Restart compilation immediately on files change"
    )]
    pub(crate) refresh: bool,

    #[arg(
        short = 'p',
        long = "postpone",
        help = "Start only after the first change"
    )]
    pub(crate) postpone: bool,

    #[arg(short = 'q', long = "quiet", help = "Suppress all compiler output")]
    pub(crate) quiet: bool,

    #[arg(
        short = 'w',
        long = "waitkey",
        help = "Wait for keypress before compilation"
    )]
    pub(crate) waitkey: bool,

    #[arg(short = 'z', long = "zen", help = "Show only commands output")]
    pub(crate) zen: bool,

    #[arg(short = 'v', long = "verbose", help = "Always print compiler output")]
    pub(crate) verbose: bool,

    #[arg(
        short = 'd',
        long = "debug",
        help = "Print the parsed arguments and exit"
    )]
    pub(crate) debug: bool,

    #[arg(value_name = "COMPILE_CMD", hide = true)]
    pub(crate) compile_cmd: Vec<String>,
}

pub(crate) enum ParseOutcome {
    Run(Cli),
    Exit { code: u8 },
}

pub(crate) fn parse(args: Vec<String>) -> anyhow::Result<ParseOutcome> {
    match Cli::try_parse_from(args) {
        Ok(cli) => Ok(ParseOutcome::Run(cli)),
        Err(err) => {
            let code = err.exit_code() as u8;
            err.print()?;
            Ok(ParseOutcome::Exit { code })
        }
    }
}
