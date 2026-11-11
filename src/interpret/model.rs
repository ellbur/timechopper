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

/// Something that happened at a moment, that may mark where one task ended
/// and another began, but says nothing by itself about what came after.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Mark {
    /// The user noted something ("starting the budget"), or just pressed
    /// the button to say "something changed here".
    Note(String),
    /// The user said they were taking a break.
    Break(String),
    /// A window was seen for the first time.
    Opened(Window),
    /// A window was closed.
    Closed(Window),
}

impl Mark {
    /// Whether the user made it, as opposed to it being observed.
    pub fn by_user(&self) -> bool {
        matches!(self, Mark::Note(_) | Mark::Break(_))
    }

    /// Whether it counts as evidence that a task ended here. A window being
    /// opened doesn't: new windows open all the time within a task.
    pub fn ends_something(&self) -> bool {
        !matches!(self, Mark::Opened(_))
    }

    /// The user's text, if any.
    pub fn text(&self) -> Option<&str> {
        match self {
            Mark::Note(t) | Mark::Break(t) if !t.is_empty() => Some(t),
            _ => None,
        }
    }
}

impl fmt::Display for Mark {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mark::Note(t) if t.is_empty() => write!(f, "marked"),
            Mark::Note(t) => write!(f, "marked \"{t}\""),
            Mark::Break(t) if t.is_empty() => write!(f, "break"),
            Mark::Break(t) => write!(f, "break: \"{t}\""),
            Mark::Opened(w) => write!(f, "opened {}", w.title),
            Mark::Closed(w) => write!(f, "closed {}", w.title),
        }
    }
}

/// A [`Mark`] and when it was made.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Marker {
    pub at: Time,
    pub mark: Mark,
}

/// One line of a log: a focus change, a marker, or the taking back of a
/// marker made by mistake.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    Focus(Event),
    Mark(Marker),
    /// At `at`, the user took back their marker made at `mark`.
    Unmark {
        at: Time,
        mark: Time,
    },
    /// At `at`, the user corrected the interpretation.
    Hint {
        at: Time,
        hint: super::Hint,
    },
    /// At `at`, the user took back their latest correction still standing.
    Unhint {
        at: Time,
    },
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
/// APP is `-` when nothing has focus. Blank lines, `#` comments and markers
/// are skipped.
pub fn parse_log(text: &str) -> Result<Vec<Event>, String> {
    let records = parse_records(text)?;
    let events = records.into_iter().filter_map(|r| match r {
        Record::Focus(e) => Some(e),
        _ => None,
    });
    Ok(events.collect())
}

/// Parses a log with its markers. Besides focus changes, a line may be
/// `HH:MM:SS @note TEXT`, `@break TEXT`, `@opened APP TITLE`,
/// `@closed APP TITLE`, or `@unmark HH:MM:SS` (taking back the user's marker
/// made at that time), `@hint FIELDS` (a correction, as
/// [`super::Hint::encode`] writes it) or `@unhint` (the latest correction
/// taken back). Focus changes must be in time order.
pub fn parse_records(text: &str) -> Result<Vec<Record>, String> {
    let mut records = Vec::new();
    let mut events: Vec<Event> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let err = |what: &str| format!("line {}: {what}: {line}", n + 1);
        let (time, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let at = Time::parse(time).ok_or_else(|| err("bad time"))?;
        let rest = rest.trim_start();
        let (app, title) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let title = title.trim();
        let window = || {
            let (app, title) = title.split_once(char::is_whitespace).unwrap_or((title, ""));
            Window {
                app: app.to_string(),
                title: title.trim().to_string(),
            }
        };
        let mark = match app {
            "@note" => Some(Mark::Note(title.to_string())),
            "@break" => Some(Mark::Break(title.to_string())),
            "@opened" => Some(Mark::Opened(window())),
            "@closed" => Some(Mark::Closed(window())),
            "@hint" => {
                // Fields are separated by tabs: take the rest of the line
                // as it is.
                let fields = rest.split_once(char::is_whitespace).map_or("", |(_, f)| f);
                let hint = super::Hint::decode(fields).map_err(|e| err(&e))?;
                records.push(Record::Hint { at, hint });
                continue;
            }
            "@unhint" => {
                records.push(Record::Unhint { at });
                continue;
            }
            "@unmark" => {
                let mark = Time::parse(title).ok_or_else(|| err("bad time to unmark"))?;
                records.push(Record::Unmark { at, mark });
                continue;
            }
            a if a.starts_with('@') => return Err(err("unknown marker")),
            _ => None,
        };
        if let Some(mark) = mark {
            records.push(Record::Mark(Marker { at, mark }));
            continue;
        }
        if events.last().is_some_and(|prev| prev.at > at) {
            return Err(err("time goes backwards"));
        }
        let window = match app {
            "" => return Err(err("missing app")),
            "-" => None,
            _ => Some(Window {
                app: app.to_string(),
                title: title.trim().to_string(),
            }),
        };
        events.push(Event { at, window });
        records.push(Record::Focus(events[events.len() - 1].clone()));
    }
    Ok(records)
}

/// One line of a log, as [`parse_records`] reads it.
pub fn record_line(record: &Record) -> String {
    // An app name with spaces would end early when read back.
    let window = |w: &Window| {
        let app = w.app.replace(char::is_whitespace, "_");
        let title = w.title.replace(['\r', '\n'], " ");
        format!("{app} {title}")
    };
    let text = |t: &str| t.replace(['\r', '\n'], " ");
    match record {
        Record::Focus(Event { at, window: None }) => format!("{at} -\n"),
        Record::Focus(Event {
            at,
            window: Some(w),
        }) => format!("{at} {}\n", window(w)),
        Record::Mark(Marker { at, mark }) => match mark {
            Mark::Note(t) => format!("{at} @note {}\n", text(t)),
            Mark::Break(t) => format!("{at} @break {}\n", text(t)),
            Mark::Opened(w) => format!("{at} @opened {}\n", window(w)),
            Mark::Closed(w) => format!("{at} @closed {}\n", window(w)),
        },
        Record::Unmark { at, mark } => format!("{at} @unmark {mark}\n"),
        Record::Hint { at, hint } => format!("{at} @hint {}\n", hint.encode()),
        Record::Unhint { at } => format!("{at} @unhint\n"),
    }
}

/// The markers left standing once those taken back are removed.
pub fn markers(records: &[Record]) -> Vec<Marker> {
    let mut out: Vec<Marker> = Vec::new();
    for record in records {
        match record {
            Record::Mark(m) => out.push(m.clone()),
            Record::Unmark { mark, .. } => {
                if let Some(i) = out.iter().rposition(|m| m.at == *mark && m.mark.by_user()) {
                    out.remove(i);
                }
            }
            _ => {}
        }
    }
    out.sort_by_key(|m| m.at);
    out
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
    fn markers_round_trip_and_can_be_taken_back() {
        let window = Window {
            app: "notepad.exe".into(),
            title: "a.txt - Notepad".into(),
        };
        let records = vec![
            Record::Mark(Marker {
                at: Time(5),
                mark: Mark::Note("budget".into()),
            }),
            Record::Focus(Event {
                at: Time(10),
                window: Some(window.clone()),
            }),
            Record::Mark(Marker {
                at: Time(10),
                mark: Mark::Opened(window.clone()),
            }),
            Record::Mark(Marker {
                at: Time(20),
                mark: Mark::Break(String::new()),
            }),
            Record::Mark(Marker {
                at: Time(30),
                mark: Mark::Closed(window),
            }),
            Record::Focus(Event {
                at: Time(30),
                window: None,
            }),
            Record::Unmark {
                at: Time(40),
                mark: Time(20),
            },
        ];
        let text: String = records.iter().map(record_line).collect();
        assert_eq!(parse_records(&text).unwrap(), records);
        assert_eq!(parse_log(&text).unwrap().len(), 2);
        let left = markers(&records);
        assert_eq!(left.len(), 3);
        assert!(left.iter().all(|m| m.at != Time(20)));
        assert!(parse_records("09:00:00 @frobnicate x").is_err());
    }

    #[test]
    fn corrections_round_trip() {
        use super::super::{Hint, Lens, Pattern};
        let t = |s| Time::parse(s).unwrap();
        let hints = [
            Hint::Lens {
                app: "Code.exe".into(),
                lens: Lens::Title,
            },
            Hint::apart(Pattern::parse("firefox:inbox (3) - gmail").unwrap()),
            Hint::Apart {
                pattern: Pattern::parse(":weekly sync: notes").unwrap(),
                during: vec![(t("09:00"), t("09:30:15")), (t("11:00"), t("11:05"))],
            },
            Hint::Together(
                Pattern::parse("notepad").unwrap(),
                Pattern::parse("firefox:quarterly report").unwrap(),
            ),
            Hint::Fixed {
                name: Some("Q3 report".into()),
                intervals: vec![(t("09:00"), t("09:24"))],
            },
            Hint::Fixed {
                name: None,
                intervals: vec![(t("10:00"), t("10:10"))],
            },
            Hint::Split { at: t("09:33:20") },
            Hint::Join {
                from: t("10:56"),
                into: t("09:33:20"),
            },
            Hint::Name {
                at: t("09:00"),
                name: "Budget: Q3, second pass".into(),
            },
            Hint::Name {
                at: t("09:00"),
                name: String::new(),
            },
            Hint::Parts {
                at: t("09:48"),
                parts: vec![
                    (Pattern::parse("winword").unwrap(), "Drafting".into()),
                    (
                        Pattern::parse("firefox:westlaw").unwrap(),
                        "Research: cases".into(),
                    ),
                ],
            },
            Hint::Parts {
                at: t("09:48"),
                parts: Vec::new(),
            },
            Hint::Combine {
                from: t("09:00"),
                into: t("09:48"),
            },
            Hint::UsualParts {
                activity: super::super::Activity {
                    app: "winword".into(),
                    subject: Some("brief.docx".into()),
                },
                parts: vec![(Pattern::parse("winword").unwrap(), "Drafting".into())],
            },
            Hint::Usual {
                activity: super::super::Activity {
                    app: "code".into(),
                    subject: None,
                },
                name: "Coding".into(),
            },
        ];
        let mut records: Vec<Record> = hints
            .iter()
            .map(|hint| Record::Hint {
                at: t("12:00"),
                hint: hint.clone(),
            })
            .collect();
        records.push(Record::Unhint { at: t("12:01") });
        let text: String = records.iter().map(record_line).collect();
        assert_eq!(parse_records(&text).unwrap(), records);
        // Tabs and line breaks in a name don't break the line apart.
        let odd = Hint::Name {
            at: t("09:00"),
            name: "a\tb\nc".into(),
        };
        let line = record_line(&Record::Hint {
            at: t("12:00"),
            hint: odd,
        });
        let back = parse_records(&line).unwrap();
        let expected = Hint::Name {
            at: t("09:00"),
            name: "a b c".into(),
        };
        assert_eq!(
            back,
            [Record::Hint {
                at: t("12:00"),
                hint: expected
            }]
        );
        assert!(parse_records("12:00:00 @hint split\tnoon").is_err());
    }

    #[test]
    fn log_errors() {
        assert!(parse_log("9am notepad.exe x").is_err());
        assert!(parse_log("09:00:00").is_err());
        assert!(parse_log("09:00:00 a.exe x\n08:00:00 b.exe y").is_err());
    }
}
