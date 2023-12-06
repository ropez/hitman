use eyre::{Result, bail};
use clap::Parser;

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
    #[arg(short, long)]
    pub select: bool,

    /// Show more output
    #[arg(short, long)]
    pub verbose: bool,

    /// Show no output except the returned data
    #[arg(short, long)]
    pub quiet: bool,
}

/// Parse a single key-value pair
fn parse_key_val(s: &str) -> Result<(String, String)>
{
    match s.find('=') {
        Some(0) => bail!("empty key in `{s}`"),
        Some(pos) => Ok((s[..pos].trim().to_string(), s[pos + 1..].trim().to_string())),
        None => bail!("no `=` found in `{s}`"),
    }
}

pub fn parse_args() -> Args {
    let args = Args::parse();
    
    args
}
