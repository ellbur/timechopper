//! What connects a source of focus changes to the journal while the day is
//! still going on: the observations themselves, a clock to read "now" from,
//! a replay of a synthetic log for trying things out without a desktop, and
//! the log file that keeps the day's observations across restarts.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crate::interpret::model::{self, Event, Span, Time, Window};

/// Focus moved to `window` at `at` (or to nothing: idle, locked).
pub type Observation = Event;

/// Where the time of day comes from.
pub trait Clock: Send + Sync {
    fn now(&self) -> Time;
}

/// A clock given as a function, e.g. the system's local time of day.
pub struct FnClock<F>(pub F);

impl<F: Fn() -> Time + Send + Sync> Clock for FnClock<F> {
    fn now(&self) -> Time {
        (self.0)()
    }
}

/// A clock that starts at `start` and runs `speed` times faster than real
/// time, stopping at `end`.
pub struct ReplayClock {
    origin: Instant,
    start: Time,
    end: Time,
    speed: f64,
}

impl ReplayClock {
    pub fn new(start: Time, end: Time, speed: f64) -> ReplayClock {
        ReplayClock {
            origin: Instant::now(),
            start,
            end,
            speed,
        }
    }

    /// How long from now (in real time) until the clock shows `at`.
    fn until(&self, at: Time) -> Duration {
        let wanted = at.0.saturating_sub(self.start.0) as f64 / self.speed;
        Duration::from_secs_f64(wanted).saturating_sub(self.origin.elapsed())
    }
}

impl Clock for ReplayClock {
    fn now(&self) -> Time {
        let gone = self.origin.elapsed().as_secs_f64() * self.speed;
        Time((self.start.0 + gone as u64).min(self.end.0))
    }
}

/// Plays a log back as if it were happening: each focus change arrives on
/// the returned channel when the returned clock reaches it. `wake` is called
/// after each one, so that a UI waiting for input can redraw.
pub fn replay(
    spans: Vec<Span>,
    speed: f64,
    wake: impl Fn() + Send + 'static,
) -> (Arc<ReplayClock>, Receiver<Observation>) {
    let start = spans.first().map_or(Time(0), |s| s.start);
    let end = spans.last().map_or(start, |s| s.end);
    let clock = Arc::new(ReplayClock::new(start, end, speed));
    let (send, receive) = mpsc::channel();
    let timer = clock.clone();
    std::thread::spawn(move || {
        for event in events(&spans) {
            std::thread::sleep(timer.until(event.at));
            if send.send(event).is_err() {
                return;
            }
            wake();
        }
    });
    (clock, receive)
}

/// The focus changes that make up a log, idle stretches included.
pub fn events(spans: &[Span]) -> Vec<Event> {
    let mut out = Vec::new();
    let mut free: Option<Time> = None;
    for span in spans {
        if free.is_some_and(|f| f < span.start) {
            out.push(Event {
                at: free.unwrap_or(span.start),
                window: None,
            });
        }
        out.push(Event {
            at: span.start,
            window: Some(span.window.clone()),
        });
        free = Some(span.end);
    }
    if let Some(at) = free {
        out.push(Event { at, window: None });
    }
    out
}

/// One line of a focus log, as [`model::parse_log`] reads it.
pub fn log_line(event: &Event) -> String {
    match &event.window {
        None => format!("{} -\n", event.at),
        Some(Window { app, title }) => {
            // An app name with spaces would end early when read back.
            let app = app.replace(char::is_whitespace, "_");
            let title = title.replace(['\r', '\n'], " ");
            format!("{} {app} {title}\n", event.at)
        }
    }
}

/// A focus log being written as things happen. Opening one that exists
/// carries on from where it left off.
pub struct LogFile {
    path: PathBuf,
    file: File,
}

impl LogFile {
    /// Opens (creating if need be) the log at `path`, and returns what was
    /// in it already.
    pub fn open(path: &Path) -> io::Result<(LogFile, Vec<Event>)> {
        let earlier = match std::fs::read_to_string(path) {
            Ok(text) => model::parse_log(&text)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let path = path.to_path_buf();
        Ok((LogFile { path, file }, earlier))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write(&mut self, event: &Event) -> io::Result<()> {
        self.file.write_all(log_line(event).as_bytes())?;
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpret::scenarios;

    #[test]
    fn events_round_trip_through_the_log_format() {
        for scenario in &scenarios::ALL {
            let spans = (scenario.spans)();
            let text: String = events(&spans).iter().map(log_line).collect();
            let back = model::spans(&model::parse_log(&text).unwrap());
            assert!(back == spans, "{}", scenario.name);
        }
    }

    #[test]
    fn a_log_file_carries_on_where_it_left_off() {
        let dir = std::env::temp_dir().join(format!("timechopper-test-{}", std::process::id()));
        let path = dir.join("day.log");
        let _ = std::fs::remove_file(&path);
        let window = |title: &str| {
            Some(Window {
                app: "notepad.exe".into(),
                title: title.into(),
            })
        };
        let first = [
            Event {
                at: Time(10),
                window: window("a.txt - Notepad"),
            },
            Event {
                at: Time(20),
                window: None,
            },
        ];
        let (mut log, earlier) = LogFile::open(&path).unwrap();
        assert!(earlier.is_empty());
        for e in &first {
            log.write(e).unwrap();
        }
        drop(log);
        let (mut log, earlier) = LogFile::open(&path).unwrap();
        assert_eq!(earlier, first);
        log.write(&Event {
            at: Time(30),
            window: window("b\nc"),
        })
        .unwrap();
        let (_, all) = LogFile::open(&path).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[2].window, window("b c"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replay_clock_runs_fast_and_stops_at_the_end() {
        let clock = ReplayClock::new(Time(100), Time(200), 1000.0);
        assert!(clock.now() >= Time(100));
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(clock.now(), Time(200));
        assert_eq!(clock.until(Time(150)), Duration::ZERO);
    }
}
