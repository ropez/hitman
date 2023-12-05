use log::{
    Level, 
    LevelFilter,
    Log, 
    Metadata, 
    Record, 
    SetLoggerError,
    set_boxed_logger, 
    set_max_level, 
};

pub struct Logger {
    level: Level,
}

pub fn init(verbose: bool, quiet: bool) -> Result<(), SetLoggerError> {
    let logger = Logger {
        level: match (verbose, quiet) {
            (true, false) => Level::Debug,
            (false, true) => Level::Error,
            _ => Level::Info,
        }
    };

    set_boxed_logger(Box::new(logger))?;
    set_max_level(LevelFilter::Info);

    Ok(())
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            eprintln!("{}", record.args());
        }
    }

    fn flush(&self) {}
}
