use log::{
    set_boxed_logger, set_max_level, Level, LevelFilter, Log, Metadata, Record, SetLoggerError,
};
use std::{
    io::{self, IsTerminal, Write},
    ops::{Deref, DerefMut},
};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

struct Logger {
    level: Level,
    color: ColorChoice,
}

/// Applies colors based on line prefices such as <, > or #,
/// while level doesn't affect the color.
impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let mut stream = ScopedColorStream::new(self.color);
            let msg = format!("{}", record.args());
            if msg.starts_with('<') {
                stream
                    .set_color(ColorSpec::new().set_fg(Some(Color::Cyan)))
                    .ok();
            } else if msg.starts_with('>') {
                stream
                    .set_color(ColorSpec::new().set_fg(Some(Color::Blue)))
                    .ok();
            } else if msg.starts_with('#') {
                stream
                    .set_color(ColorSpec::new().set_fg(Some(Color::Yellow)))
                    .ok();
            }
            writeln!(&mut stream, "{}", record.args()).unwrap_or_else(|_| {
                eprintln!("{}", record.args());
            });
        }
    }

    fn flush(&self) {
        io::stderr().flush().ok();
    }
}

pub fn init(verbose: bool, quiet: bool, batch: bool) -> Result<(), SetLoggerError> {
    let logger = Logger {
        level: match (verbose, quiet, batch) {
            (_, true, _) => Level::Error,
            (_, _, true) => Level::Warn,
            (true, _, _) => Level::Debug,
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

/// Wrapper that automatically resets the terminal color
struct ScopedColorStream {
    stream: StandardStream,
}

impl ScopedColorStream {
    fn new(color: ColorChoice) -> Self {
        Self {
            stream: StandardStream::stderr(color),
        }
    }
}

impl Drop for ScopedColorStream {
    fn drop(&mut self) {
        self.stream.reset().ok();
    }
}

impl Deref for ScopedColorStream {
    type Target = StandardStream;

    fn deref(&self) -> &Self::Target {
        &self.stream
    }
}

impl DerefMut for ScopedColorStream {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.stream
    }
}
