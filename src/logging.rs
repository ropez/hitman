use log::{
    set_boxed_logger, set_max_level, Level, LevelFilter, Log, Metadata, Record, SetLoggerError,
};
use std::io::{self, IsTerminal, Write};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

struct Logger {
    level: Level,
    color: ColorChoice,
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let mut stream: StandardStream = StandardStream::stderr(self.color);
            let msg = format!("{}", record.args());
            if msg.starts_with("<") {
                stream
                    .set_color(ColorSpec::new().set_fg(Some(Color::Cyan)))
                    .ok();
            } else if msg.starts_with(">") {
                stream
                    .set_color(ColorSpec::new().set_fg(Some(Color::Blue)))
                    .ok();
            } else if msg.starts_with("#") {
                stream
                    .set_color(ColorSpec::new().set_fg(Some(Color::Yellow)))
                    .ok();
            }
            writeln!(&mut stream, "{}", record.args()).unwrap_or_else(|_| {
                eprintln!("{}", record.args());
            });
            stream.reset().ok();
        }
    }

    fn flush(&self) {
        io::stderr().flush().ok();
    }
}

pub fn init(verbose: bool, quiet: bool) -> Result<(), SetLoggerError> {
    let logger = Logger {
        level: match (verbose, quiet) {
            (true, false) => Level::Debug,
            (false, true) => Level::Error,
            _ => Level::Info,
        },
        color: if io::stderr().is_terminal() {
            ColorChoice::Auto
        } else {
            ColorChoice::Never
        },
    };

    set_boxed_logger(Box::new(logger))?;
    set_max_level(LevelFilter::Info);

    Ok(())
}
