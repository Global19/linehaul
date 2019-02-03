use std::env;
use std::error::Error;
use std::io;
use std::io::prelude::*;
use std::str;

#[macro_use]
extern crate lazy_static;

#[macro_use]
extern crate nom;

use flate2::read::GzDecoder;
use slog;
use slog::{o, slog_error, slog_trace, slog_warn, Drain};
use slog_async;
use slog_envlogger;
use slog_scope;
use slog_scope::{error, trace, warn};
use slog_term;

mod events;
mod syslog;
mod ua;

#[allow(dead_code)]
mod build {
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

pub enum LogStyle {
    JSON,
    Readable,
}

pub fn default_logger(style: LogStyle) -> slog::Logger {
    let level = match env::var("LINEHAUL_LOG") {
        Ok(s) => s.to_string(),
        Err(_e) => "debug".to_string(),
    };
    let kv = o!("version" => build::PKG_VERSION, "commit" => build::GIT_VERSION);

    match style {
        LogStyle::JSON => {
            let drain = slog_bunyan::default(io::stdout()).fuse();
            let drain = slog_envlogger::LogBuilder::new(drain)
                .parse(level.as_ref())
                .build();
            let drain = slog_async::Async::new(drain).build().fuse();

            slog::Logger::root(drain, kv)
        }
        LogStyle::Readable => {
            let decorator = slog_term::TermDecorator::new().stdout().build();
            let drain = slog_term::CompactFormat::new(decorator).build().fuse();
            let drain = slog_envlogger::LogBuilder::new(drain)
                .parse(level.as_ref())
                .build();
            let drain = slog_async::Async::new(drain).build().fuse();

            slog::Logger::root(drain, kv)
        }
    }
}

fn process_event(raw_event: &str) -> Option<events::Event> {
    match raw_event.parse::<events::Event>() {
        Ok(e) => Some(e),
        Err(e) => {
            match e {
                events::EventParseError::IgnoredUserAgent => {
                    trace!("skipping for ignored user agent");
                }
                events::EventParseError::InvalidUserAgent => {
                    trace!("skipping for invalid user agent");
                }
                events::EventParseError::Error => {
                    error!("invalid event");
                }
            };

            None
        }
    }
}

pub fn process<'a>(lines: impl Iterator<Item = &'a str>) {
    for line in lines {
        // Parse each line as a syslog message.
        let message: syslog::SyslogMessage = match line.parse() {
            Ok(m) => m,
            Err(_e) => {
                error!("could not parse as syslog message";
                       "line" => line);
                continue;
            }
        };

        // Parse the log entry as an event.
        let _event = slog_scope::scope(
            &slog_scope::logger().new(o!("raw_event" => message.message.clone())),
            || process_event(message.message.as_ref()),
        );
    }
}

pub fn process_reader(file: impl Read) -> Result<(), Box<dyn Error>> {
    let mut gz = GzDecoder::new(file);
    let mut buffer = Vec::new();

    gz.read_to_end(&mut buffer)?;

    let lines = buffer
        .split(|c| c == &b'\n')
        .into_iter()
        .filter_map(|line| match str::from_utf8(line) {
            Ok(l) => Some(l),
            Err(e) => {
                warn!("skipping invalid line";
                      "line" => String::from_utf8_lossy(line).as_ref(),
                      "error" => e.to_string());
                None
            }
        })
        .filter(|i| i.len() > 0);

    process(lines);

    Ok(())
}
