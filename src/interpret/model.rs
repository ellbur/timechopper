//! The raw material: which window had focus, and when.

use std::fmt;

/// A point in time, in seconds since midnight (or any other epoch).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time(pub u64);

impl Time {
    pub fn hms(h: u64, m: u64, s: u64) -> Time {
        Time(h * 3600 + m * 60 + s)
    }

    /// Parses `HH:MM` or `HH:MM:SS`.
    pub fn parse(text: &str) -> Option<Time> {
        let mut parts = text.split(':').map(|p| p.parse::<u64>().ok());
        let h = parts.next()??;
        let m = parts.next()??;
        let s = match parts.next() {
            Some(s) => s?,
            None => 0,
        };
        (parts.next().is_none() && m < 60 && s < 60).then(|| Time::hms(h, m, s))
    }

    /// `HH:MM`, for display where seconds would be noise.
    pub fn hm(self) -> String {
        format!("{:02}:{:02}", self.0 / 3600, self.0 / 60 % 60)
    }
}

impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{:02}", self.hm(), self.0 % 60)
    }
}

/// `1h27m`, `5m10s`, `42s`.
pub fn fmt_duration(secs: u64) -> String {
    match secs {
        3600.. => format!("{}h{:02}m", secs / 3600, secs / 60 % 60),
        60.. if secs.is_multiple_of(60) => format!("{}m", secs / 60),
        60.. => format!("{}m{:02}s", secs / 60, secs % 60),
        _ => format!("{secs}s"),
    }
}

/// What is known about a focused window. `app` is the executable's file name
/// (`notepad.exe`); a path is accepted too.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Window {
    pub app: String,
    pub title: String,
}

/// Focus moved to `window` at `at`. `None` means nothing was being used
/// (idle, locked, logged out): that time belongs to no task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub at: Time,
    pub window: Option<Window>,
}

/// One window holding focus from `start` to `end`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: Time,
    pub end: Time,
    pub window: Window,
}

impl Span {
    pub fn secs(&self) -> u64 {
        self.end.0 - self.start.0
    }
}

/// Turns focus events into spans: each window holds focus until the next
/// event. The last event only marks the end of the log, so a log normally
/// ends with a `None` event. Events must be in time order.
pub fn spans(events: &[Event]) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    for pair in events.windows(2) {
        let (Some(window), end) = (&pair[0].window, pair[1].at) else {
            continue;
        };
        let start = pair[0].at;
        if end <= start {
            continue;
        }
        match out.last_mut() {
            Some(prev) if prev.end == start && prev.window == *window => prev.end = end,
            _ => out.push(Span {
                start,
                end,
                window: window.clone(),
            }),
        }
    }
    out
}

/// Parses a focus log: one event per line, `HH:MM:SS APP TITLE...`, where
/// APP is `-` when nothing has focus. Blank lines and `#` comments are
/// skipped.
pub fn parse_log(text: &str) -> Result<Vec<Event>, String> {
    let mut events: Vec<Event> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let err = |what: &str| format!("line {}: {what}: {line}", n + 1);
        let (time, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let at = Time::parse(time).ok_or_else(|| err("bad time"))?;
        if events.last().is_some_and(|prev| prev.at > at) {
            return Err(err("time goes backwards"));
        }
        let rest = rest.trim_start();
        let (app, title) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let window = match app {
            "" => return Err(err("missing app")),
            "-" => None,
            _ => Some(Window {
                app: app.to_string(),
                title: title.trim().to_string(),
            }),
        };
        events.push(Event { at, window });
    }
    Ok(events)
}

/// The inverse of [`parse_log`].
pub fn format_log(spans: &[Span]) -> String {
    let mut out = String::new();
    let mut end = None;
    for span in spans {
        if end.is_some_and(|end| end != span.start) {
            out.push_str(&format!("{} -\n", end.unwrap()));
        }
        let Window { app, title } = &span.window;
        out.push_str(&format!("{} {app} {title}\n", span.start));
        end = Some(span.end);
    }
    if let Some(end) = end {
        out.push_str(&format!("{end} -\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(fmt_duration(42), "42s");
        assert_eq!(fmt_duration(300), "5m");
        assert_eq!(fmt_duration(310), "5m10s");
        assert_eq!(fmt_duration(5225), "1h27m");
    }

    #[test]
    fn log_round_trip() {
        let text = "\
# a comment
09:00:00 notepad.exe report.txt - Notepad
09:02:00 firefox.exe Some page — Mozilla Firefox
09:03:00 -
09:10:00 notepad.exe report.txt - Notepad
09:11:30 -
";
        let spans = spans(&parse_log(text).unwrap());
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[1].window.title, "Some page — Mozilla Firefox");
        assert_eq!(spans[2].secs(), 90);
        assert_eq!(format_log(&spans), text.split_once('\n').unwrap().1);
    }

    #[test]
    fn log_errors() {
        assert!(parse_log("9am notepad.exe x").is_err());
        assert!(parse_log("09:00:00").is_err());
        assert!(parse_log("09:00:00 a.exe x\n08:00:00 b.exe y").is_err());
    }
}
