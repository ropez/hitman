use clap::Parser;
use eyre::{bail, Result};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// The name of a request file to execute and exit.
    /// Omit this argument to run an interactive prompt.
    pub name: Option<String>,

    /// Optional Name=Value pairs to substitute in the request.
    /// These will override values in the config file.
    #[arg(value_parser = parse_key_val)]
    pub options: Vec<(String, String)>,

    /// When running interactively (no name argument specified),
    /// repeat asking for requests until cancelled.
    #[arg(short, long)]
    pub repeat: bool,

    /// Select a target from the config file
    #[arg(short, long, conflicts_with = "name", conflicts_with = "repeat")]
    pub select: bool,

    /// Show more output
    #[arg(short, long)]
    pub verbose: bool,

    /// Show no output except the returned data
    #[arg(short, long)]
    pub quiet: bool,

    /// Do not ask questions
    #[arg(short, long, requires = "name")]
    pub non_interactive: bool,

    /// Number of requests to send in batch
    #[arg(long, conflicts_with = "repeat", requires = "name")]
    pub batch: Option<i32>,

    /// Number of requests to send in batch
    #[arg(short, long, requires = "batch")]
    pub connections: Option<i32>,

    /// Watch file for changes (implies non-interactove)
    #[arg(short, long, requires = "name", conflicts_with = "batch")]
    pub watch: bool,
}

/// Parse a single key-value pair
fn parse_key_val(s: &str) -> Result<(String, String)> {
    match s.find('=') {
        Some(0) => bail!("empty key in `{s}`"),
        Some(pos) => Ok((s[..pos].trim().to_string(), s[pos + 1..].trim().to_string())),
        None => bail!("no `=` found in `{s}`"),
    }
}

pub fn parse_args() -> Args {
    Args::parse()
}
