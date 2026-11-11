//! Interpreting a focus log: turning "which window had focus when" into a
//! short list of what the user was probably working on.
//!
//! The pieces, in the order they are applied:
//!
//! * [`title`] decides what each window *is*: its [`Activity`]. How much of
//!   the title counts is a choice (a [`Lens`]), guessed per window and
//!   overridable per app.
//! * `structure` partitions the visits to those activities into tasks, by
//!   the pattern of going back and forth between them.
//! * This module puts the result on the clock as a [`Timeline`]: every task
//!   gets one or more [`Session`]s, and a session that falls inside a pause
//!   of another is a carve-out of it.
//!
//! Every second of the log belongs to exactly one task, so times always add
//! up however the grouping is changed. A session's `active` time is what was
//! spent on it; the rest of its extent is time carved out for something
//! else, which is counted there.
//!
//! The grouping is a guess, and the timeline says where the guess is thin:
//! each session lists its [`Member`]s with the reason each is there and any
//! [`Doubt`] about it, and the [`PossibleSplit`]s it stopped short of.
//!
//! [`Hint`]s are how the user corrects the guess. [`interpret`] is a pure
//! function of the log and the hints; [`journal::Journal`] holds both for a
//! log that is still growing, and turns "do this to that row" into hints.

pub mod history;
pub mod journal;
pub mod model;
pub mod render;
pub mod scenarios;
mod structure;
pub mod title;

use std::collections::{BTreeMap, HashMap, HashSet};

pub use journal::{Diff, Journal, Scope};
pub use model::{Event, Mark, Marker, Record, Span, Time, Window};
pub use title::{Activity, Lens, Pattern};

use structure::Visit;
use title::Titles;

/// A correction to the guessed interpretation.
///
/// Hints refer to the log by time and by pattern, never by row, so they keep
/// their meaning as the log grows and as other hints come and go. The first
/// four shape the guess itself; `Split`, `Join` and `Name` are applied to
/// its result, in the order given, and leave everything they don't mention
/// as it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hint {
    /// How much of this app's titles counts towards identity.
    Lens { app: String, lens: Lens },
    /// These windows are a task of their own, whatever they alternate with:
    /// always, or (if `during` is not empty) only within those intervals.
    Apart {
        pattern: Pattern,
        during: Vec<(Time, Time)>,
    },
    /// Windows matching either pattern are the same activity, so they are
    /// never the reason to tell two tasks apart.
    Together(Pattern, Pattern),
    /// Whatever had focus during these intervals is one session of one task,
    /// exactly as given: a row the user has confirmed. It is left out of the
    /// guessing and is unaffected by any other hint.
    Fixed {
        name: Option<String>,
        intervals: Vec<(Time, Time)>,
    },
    /// The task in focus at this time became a different task at this time:
    /// what it does from here on is counted separately. `at` may fall in the
    /// middle of a window's time in focus.
    Split { at: Time },
    /// The session in focus at `from` belongs to the task in focus at `into`.
    Join { from: Time, into: Time },
    /// What the user calls the task in focus at this time. Tasks given the
    /// same name are the same task. An empty name takes away the name the
    /// task had (one carried over from earlier days, say).
    Name { at: Time, name: String },
    /// A standing rule, learned from earlier days: a task whose
    /// [`Task::identity`] is this activity is called this. Applied before
    /// any [`Hint::Name`], which overrides it.
    Usual { activity: Activity, name: String },
    /// The task in focus at `at` is divided into parts: time on windows
    /// matching a pattern counts towards the part named with it (the first
    /// pattern that matches decides); anything else is "other". Parts are
    /// shares of the task's time, not stretches of it: the rows stay as
    /// they are. The whole division is given at once, replacing any before
    /// it; an empty one undoes the division.
    Parts {
        at: Time,
        parts: Vec<(Pattern, String)>,
    },
    /// The whole task in focus at `from` is part of the task in focus at
    /// `into`: one task, in which each keeps its time as a part.
    Combine { from: Time, into: Time },
    /// A standing rule, learned from earlier days: a task whose identity is
    /// this activity is divided this way, unless divided otherwise today.
    UsualParts {
        activity: Activity,
        parts: Vec<(Pattern, String)>,
    },
}

impl Hint {
    /// The hint as one line of text, for keeping in a log: a keyword and
    /// fields separated by tabs (free text has its tabs and line breaks
    /// turned to spaces). [`Hint::decode`] reads it back.
    pub fn encode(&self) -> String {
        let clean = |t: &str| t.replace(['\t', '\r', '\n'], " ");
        let ranges = |rs: &[(Time, Time)]| {
            let parts: Vec<String> = rs.iter().map(|(a, b)| format!("{a}-{b}")).collect();
            match parts.is_empty() {
                true => "-".to_string(),
                false => parts.join(","),
            }
        };
        let fields: Vec<String> = match self {
            Hint::Lens { app, lens } => {
                let lens = match lens {
                    Lens::Auto => "auto",
                    Lens::App => "app",
                    Lens::Title => "title",
                };
                vec!["lens".into(), clean(app), lens.into()]
            }
            Hint::Apart { pattern, during } => {
                vec!["apart".into(), ranges(during), clean(&pattern.to_string())]
            }
            Hint::Together(a, b) => vec![
                "together".into(),
                clean(&a.to_string()),
                clean(&b.to_string()),
            ],
            Hint::Fixed { name, intervals } => vec![
                "fixed".into(),
                ranges(intervals),
                clean(name.as_deref().unwrap_or("")),
            ],
            Hint::Split { at } => vec!["split".into(), at.to_string()],
            Hint::Join { from, into } => vec!["join".into(), from.to_string(), into.to_string()],
            Hint::Name { at, name } => vec!["name".into(), at.to_string(), clean(name)],
            Hint::Usual { activity, name } => vec![
                "usual".into(),
                clean(&activity.app),
                clean(activity.subject.as_deref().unwrap_or("")),
                clean(name),
            ],
            Hint::Parts { at, parts } => {
                let mut fields = vec!["parts".into(), at.to_string()];
                for (pattern, name) in parts {
                    fields.extend([clean(&pattern.to_string()), clean(name)]);
                }
                fields
            }
            Hint::Combine { from, into } => {
                vec!["combine".into(), from.to_string(), into.to_string()]
            }
            Hint::UsualParts { activity, parts } => {
                let mut fields = vec![
                    "usualparts".into(),
                    clean(&activity.app),
                    clean(activity.subject.as_deref().unwrap_or("")),
                ];
                for (pattern, name) in parts {
                    fields.extend([clean(&pattern.to_string()), clean(name)]);
                }
                fields
            }
        };
        fields.join("\t")
    }

    /// Reads a hint written by [`Hint::encode`].
    pub fn decode(text: &str) -> Result<Hint, String> {
        let fields: Vec<&str> = text.split('\t').collect();
        let err = || format!("bad correction: {text}");
        let time = |t: &str| Time::parse(t).ok_or_else(err);
        let ranges = |t: &str| -> Result<Vec<(Time, Time)>, String> {
            if t == "-" {
                return Ok(Vec::new());
            }
            t.split(',')
                .map(|r| {
                    let (a, b) = r.split_once('-').ok_or_else(err)?;
                    Ok((time(a)?, time(b)?))
                })
                .collect()
        };
        let pairs = |rest: &[&str]| -> Result<Vec<(Pattern, String)>, String> {
            rest.chunks(2)
                .map(|pair| match pair {
                    [pattern, name] => Ok((Pattern::parse(pattern)?, name.to_string())),
                    _ => Err(err()),
                })
                .collect()
        };
        match fields[..] {
            ["parts", at, ref rest @ ..] => {
                return Ok(Hint::Parts {
                    at: time(at)?,
                    parts: pairs(rest)?,
                });
            }
            ["usualparts", app, subject, ref rest @ ..] => {
                return Ok(Hint::UsualParts {
                    activity: Activity {
                        app: app.to_string(),
                        subject: (!subject.is_empty()).then(|| subject.to_string()),
                    },
                    parts: pairs(rest)?,
                });
            }
            _ => {}
        }
        Ok(match fields[..] {
            ["combine", from, into] => Hint::Combine {
                from: time(from)?,
                into: time(into)?,
            },
            ["lens", app, lens] => Hint::Lens {
                app: app.to_string(),
                lens: match lens {
                    "auto" => Lens::Auto,
                    "app" => Lens::App,
                    "title" => Lens::Title,
                    _ => return Err(err()),
                },
            },
            ["apart", during, pattern] => Hint::Apart {
                pattern: Pattern::parse(pattern)?,
                during: ranges(during)?,
            },
            ["together", a, b] => Hint::Together(Pattern::parse(a)?, Pattern::parse(b)?),
            // An empty name at the end of a line may have been trimmed off.
            ["fixed", intervals] => Hint::Fixed {
                name: None,
                intervals: ranges(intervals)?,
            },
            ["fixed", intervals, name] => Hint::Fixed {
                name: (!name.is_empty()).then(|| name.to_string()),
                intervals: ranges(intervals)?,
            },
            ["split", at] => Hint::Split { at: time(at)? },
            ["join", from, into] => Hint::Join {
                from: time(from)?,
                into: time(into)?,
            },
            // An empty name at the end of a line may have been trimmed off.
            ["name", at] => Hint::Name {
                at: time(at)?,
                name: String::new(),
            },
            ["usual", app, subject, name] => Hint::Usual {
                activity: Activity {
                    app: app.to_string(),
                    subject: (!subject.is_empty()).then(|| subject.to_string()),
                },
                name: name.to_string(),
            },
            ["name", at, name] => Hint::Name {
                at: time(at)?,
                name: name.to_string(),
            },
            _ => return Err(err()),
        })
    }

    /// These windows are always a task of their own.
    pub fn apart(pattern: Pattern) -> Hint {
        Hint::Apart {
            pattern,
            during: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Params {
    /// The longest time away, in seconds, that still counts as an
    /// interruption. Something left for longer and then resumed is shown as
    /// resumed, in a new session.
    pub max_gap: u64,
    /// For a stretch of work to be split in two while some activity runs
    /// through both halves, the activities found in one half only must make
    /// up at least this share of each half.
    pub min_exclusive_share: f64,
    /// A split that falls short of `min_exclusive_share` but reaches this is
    /// offered as a [`PossibleSplit`].
    pub near_miss_share: f64,
    /// How far apart, in seconds, a marker and a change of work may be and
    /// still be about the same moment.
    pub mark_tolerance: u64,
}

impl Default for Params {
    fn default() -> Params {
        Params {
            max_gap: 15 * 60,
            min_exclusive_share: 0.5,
            near_miss_share: 0.25,
            mark_tolerance: 120,
        }
    }
}

/// A title part seen during a session or task, and for how long.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evidence {
    pub text: String,
    pub secs: u64,
}

/// What a task or session seems to be about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    /// The title part focus was on the longest, whichever window it came
    /// from; the apps' names if no title said anything.
    pub headline: String,
    /// Apps involved, most used first, by the name their titles give them.
    pub apps: Vec<String>,
    /// The next most prominent title parts.
    pub also: Vec<Evidence>,
}

/// Why an activity is counted with a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// More time went into it than into anything else here.
    Main,
    /// The user kept coming back to it.
    Returned,
    /// Seen once, in an app the user kept coming back to: it may have
    /// nothing else in common with the task.
    SameApp,
    /// Seen once in this session; it is here on the strength of the task's
    /// other sessions, or of a hint.
    Once,
    /// The user confirmed the session as it is.
    Confirmed,
}

/// A reason to think an activity might not belong where it was put.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Doubt {
    /// It has an identity of its own and turns up in this many other tasks
    /// too; if `own`, it is the main activity of one. It may be a thing of
    /// its own, done on the side: mail, chat.
    Elsewhere { tasks: usize, own: bool },
}

/// One of the activities a session is made of.
#[derive(Clone, Debug)]
pub struct Member {
    pub activity: Activity,
    /// For display: the app as its titles name it, and the subject.
    pub name: String,
    pub secs: u64,
    pub visits: usize,
    pub why: Why,
    pub doubts: Vec<Doubt>,
    /// Indexes into [`Timeline::spans`].
    pub spans: Vec<usize>,
    /// Whether, on earlier days, the user took it out of the task it was
    /// swept into: filled in by [`history::Lessons::annotate`].
    pub precedent: Option<history::Precedent>,
    /// The part of the task it counts towards, if the task is divided
    /// (`None` for "other").
    pub part: Option<String>,
}

/// A point inside a session where one task may have given way to another
/// without the guess being sure enough to split it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PossibleSplit {
    pub at: Time,
    /// What the session is about up to that point only, and from it only.
    pub before: String,
    pub after: String,
}

/// Something the user was working on.
#[derive(Clone, Debug)]
pub struct Task {
    /// The name the user gave it, if any (or one carried over from earlier
    /// days: see `precedent`).
    pub name: Option<String>,
    /// What identifies it from one day to the next: the activity with a
    /// subject (a project, a site) that took the most time, if any did.
    pub identity: Option<Activity>,
    /// What earlier days say it is called, if they say anything: filled in
    /// by [`history::Lessons::annotate`].
    pub precedent: Option<history::Precedent>,
    /// The task's time divided into parts, if it is divided: they add up
    /// to `active`.
    pub parts: Vec<Part>,
    /// How earlier days divided it, if they did.
    pub division: Option<history::Precedent>,
    pub label: Label,
    /// The activities that make it up, most time first.
    pub activities: Vec<Activity>,
    /// Seconds spent on it, over all sessions.
    pub active: u64,
    /// Indexes into [`Timeline::sessions`].
    pub sessions: Vec<usize>,
}

/// A share of a task's time: what was spent on the windows counted towards
/// it, wherever they fell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub name: String,
    pub secs: u64,
}

/// A stretch of work on one task, interruptions included.
#[derive(Clone, Debug)]
pub struct Session {
    /// Index into [`Timeline::tasks`].
    pub task: usize,
    pub start: Time,
    pub end: Time,
    /// Seconds actually spent on the task between `start` and `end`.
    pub active: u64,
    /// How many separate window visits were collapsed into this session.
    pub visits: usize,
    /// The session this one is carved out of: one that was under way both
    /// before and after it, and that more time went into. Normally the one
    /// it interrupted.
    pub parent: Option<usize>,
    pub label: Label,
    /// What was swept into this session, most time first.
    pub members: Vec<Member>,
    /// Places it might be split, in time order.
    pub splits: Vec<PossibleSplit>,
    /// The user confirmed this session ([`Hint::Fixed`]).
    pub confirmed: bool,
    /// This session's time in each of its task's parts, if the task is
    /// divided.
    pub parts: Vec<Part>,
    /// The user's marker this session starts at, if there is one: an index
    /// into [`Timeline::marks`].
    pub opened_by: Option<usize>,
    /// The user's other markers made while this session was going on.
    pub marks: Vec<usize>,
    /// Indexes into [`Timeline::spans`].
    pub spans: Vec<usize>,
}

impl Session {
    /// Seconds within the session's extent spent on something else (or on
    /// nothing).
    pub fn elsewhere(&self) -> u64 {
        self.end.0 - self.start.0 - self.active
    }
}

/// An interpretation of a focus log.
#[derive(Clone, Debug, Default)]
pub struct Timeline {
    /// The log, with spans cut in two wherever a hint draws a line through
    /// one.
    pub spans: Vec<Span>,
    /// Ordered by first appearance.
    pub tasks: Vec<Task>,
    /// Ordered by start.
    pub sessions: Vec<Session>,
    /// Everything marked along the way, in time order: the user's notes and
    /// breaks, and windows opened and closed.
    pub marks: Vec<Marker>,
    /// Seconds of the log during which some window had focus. Always the sum
    /// of the tasks' (and of the sessions') active time.
    pub tracked: u64,
}

/// One activity holding focus without a break.
struct Run {
    activity: usize,
    start: Time,
    end: Time,
    spans: Vec<usize>,
    /// The [`Hint::Fixed`] (by position) that put it where it is.
    fixed: Option<usize>,
    /// The part of its task it counts towards, if the task is divided.
    part: Option<String>,
}

/// A task in the making.
#[derive(Default)]
struct Draft {
    runs: Vec<Run>,
    name: Option<String>,
}

fn within(ranges: &[(Time, Time)], t: Time) -> bool {
    ranges.iter().any(|&(from, to)| from <= t && t < to)
}

/// Cuts spans wherever a hint draws a line through one.
fn cut(spans: &[Span], hints: &[Hint]) -> Vec<Span> {
    let mut times: Vec<Time> = Vec::new();
    for hint in hints {
        match hint {
            Hint::Split { at } => times.push(*at),
            Hint::Apart { during: ranges, .. }
            | Hint::Fixed {
                intervals: ranges, ..
            } => {
                times.extend(ranges.iter().flat_map(|&(from, to)| [from, to]));
            }
            _ => {}
        }
    }
    times.sort();
    times.dedup();
    let mut out = Vec::with_capacity(spans.len());
    for span in spans {
        let mut start = span.start;
        let next = times.partition_point(|&t| t <= start);
        for &t in times[next..].iter().take_while(|&&t| t < span.end) {
            out.push(Span {
                start,
                end: t,
                window: span.window.clone(),
            });
            start = t;
        }
        out.push(Span {
            start,
            end: span.end,
            window: span.window.clone(),
        });
    }
    out
}

/// Joins consecutive spans of the same activity into runs.
fn runs(
    spans: &[Span],
    members: &[usize],
    activity: impl Fn(usize) -> usize,
    fixed: Option<usize>,
) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for &i in members {
        match runs.last_mut() {
            Some(run) if run.activity == activity(i) && run.end == spans[i].start => {
                run.end = spans[i].end;
                run.spans.push(i);
            }
            _ => runs.push(Run {
                activity: activity(i),
                start: spans[i].start,
                end: spans[i].end,
                spans: vec![i],
                fixed,
                part: None,
            }),
        }
    }
    runs
}

/// Which of a task's runs, given in time order, make up each session: a
/// confirmed row is a session as it stands; the rest is divided wherever the
/// task was left for longer than `max_gap`.
fn sessions_of(runs: &[Run], max_gap: u64) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut confirmed: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut last: Option<Time> = None;
    for (i, run) in runs.iter().enumerate() {
        if let Some(hint) = run.fixed {
            confirmed.entry(hint).or_default().push(i);
            continue;
        }
        match out.last_mut() {
            Some(session) if run.start.0 - last.unwrap().0 <= max_gap => session.push(i),
            _ => out.push(vec![i]),
        }
        last = Some(run.end);
    }
    out.extend(confirmed.into_values());
    out
}

/// The span in focus at `t`.
fn span_at(spans: &[Span], t: Time) -> Option<usize> {
    let i = spans.partition_point(|s| s.end <= t);
    (i < spans.len() && spans[i].start <= t).then_some(i)
}

/// The draft a span was put in.
fn owner(drafts: &[Draft], span: usize) -> Option<usize> {
    drafts
        .iter()
        .position(|d| d.runs.iter().any(|r| r.spans.contains(&span)))
}

/// Interprets a focus log. `spans` must be in time order and not overlap,
/// as produced by [`model::spans`].
pub fn interpret(spans: &[Span], hints: &[Hint], params: &Params) -> Timeline {
    interpret_marked(spans, &[], hints, params)
}

/// Interprets a focus log along with what was marked as it went on.
///
/// A marker is evidence that something ended there, not that something new
/// went on from there. Where the guess would only *offer* a split (a
/// [`PossibleSplit`]: the work changes, but not clearly enough), a marker at
/// that moment (the user's note or break, or a window being closed) decides
/// it. A marker where the work doesn't change cuts nothing. The user's
/// markers are listed with the rows they fall in, as the user's own account
/// of the day to reconstruct it from.
pub fn interpret_marked(
    spans: &[Span],
    marks: &[Marker],
    hints: &[Hint],
    params: &Params,
) -> Timeline {
    let mut marks = marks.to_vec();
    marks.sort_by_key(|m| m.at);
    let first = guess(spans, &marks, hints, params);
    // Possible splits with a marker at them become splits. They come before
    // the user's own corrections, so that a Join can undo one.
    let evidence = marks.iter().filter(|m| m.mark.ends_something());
    let evidence: Vec<Time> = evidence.map(|m| m.at).collect();
    let mut decided: Vec<Hint> = Vec::new();
    for session in &first.sessions {
        for split in &session.splits {
            if evidence
                .iter()
                .any(|e| e.0.abs_diff(split.at.0) <= params.mark_tolerance)
            {
                decided.push(Hint::Split { at: split.at });
            }
        }
    }
    if decided.is_empty() {
        return first;
    }
    decided.extend(hints.iter().cloned());
    guess(spans, &marks, &decided, params)
}

/// What to call a task's time when it becomes a part of another: what it
/// was mostly about.
fn part_label(runs: &[Run], activities: &[Activity], titles: &Titles) -> String {
    let mut secs: HashMap<usize, u64> = HashMap::new();
    for run in runs {
        *secs.entry(run.activity).or_default() += run.end.0 - run.start.0;
    }
    let main = secs
        .into_iter()
        .max_by_key(|&(a, s)| (s, std::cmp::Reverse(a)));
    match main.map(|(a, _)| &activities[a]) {
        Some(Activity {
            subject: Some(subject),
            ..
        }) => subject.clone(),
        Some(Activity { app, .. }) => titles.display_name(app).to_string(),
        None => String::new(),
    }
}

/// The time spent on each part, largest first, with anything not in a
/// part last, as "other". Empty if the task is not `divided`.
fn parts_of<'a>(runs: impl Iterator<Item = &'a Run>, divided: bool) -> Vec<Part> {
    let mut parts: Vec<Part> = Vec::new();
    let mut other = 0;
    for run in runs {
        let secs = run.end.0 - run.start.0;
        match &run.part {
            Some(name) => match parts.iter_mut().find(|p| p.name == *name) {
                Some(part) => part.secs += secs,
                None => parts.push(Part {
                    name: name.clone(),
                    secs,
                }),
            },
            None => other += secs,
        }
    }
    if !divided {
        return Vec::new();
    }
    parts.sort_by(|a, b| b.secs.cmp(&a.secs).then_with(|| a.name.cmp(&b.name)));
    if other > 0 {
        parts.push(Part {
            name: OTHER.to_string(),
            secs: other,
        });
    }
    parts
}

/// What the time of a divided task outside any of its parts is called.
pub const OTHER: &str = "other";

/// The activity with a subject that took the most time in these runs.
fn identity(runs: &[Run], activities: &[Activity]) -> Option<Activity> {
    let mut secs: HashMap<usize, u64> = HashMap::new();
    for run in runs {
        *secs.entry(run.activity).or_default() += run.end.0 - run.start.0;
    }
    secs.into_iter()
        .filter(|(a, _)| activities[*a].subject.is_some())
        .max_by(|(a, x), (b, y)| x.cmp(y).then_with(|| activities[*b].cmp(&activities[*a])))
        .map(|(a, _)| activities[a].clone())
}

/// One pass of interpretation; `marks` must be in time order.
fn guess(spans: &[Span], marks: &[Marker], hints: &[Hint], params: &Params) -> Timeline {
    let marks = marks.to_vec();
    let spans = cut(spans, hints);
    let spans = &spans[..];
    let titles = Titles::learn(spans);

    // What each span is, and where it is analysed: confirmed rows are taken
    // as given; of the rest, strand 0 is everything no hint sets apart.
    let mut activities: Vec<Activity> = Vec::new();
    let mut ids: HashMap<Activity, usize> = HashMap::new();
    let mut activity_of = Vec::with_capacity(spans.len());
    let mut strands: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut confirmed: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, span) in spans.iter().enumerate() {
        let window = &span.window;
        let (mut lens, mut alias, mut strand, mut fixed) = (Lens::Auto, None, None, None);
        for (h, hint) in hints.iter().enumerate() {
            match hint {
                Hint::Lens { app, lens: l } if title::app_stem(app) == titles.stem(i) => lens = *l,
                Hint::Together(a, b)
                    if alias.is_none() && (a.matches(window) || b.matches(window)) =>
                {
                    alias = Some(Activity {
                        app: format!("{a} + {b}"),
                        subject: None,
                    });
                }
                Hint::Apart { pattern, during }
                    if strand.is_none()
                        && pattern.matches(window)
                        && (during.is_empty() || within(during, span.start)) =>
                {
                    strand = Some(h + 1);
                }
                Hint::Fixed { intervals, .. }
                    if fixed.is_none() && within(intervals, span.start) =>
                {
                    fixed = Some(h);
                }
                _ => {}
            }
        }
        let activity = alias.unwrap_or_else(|| titles.activity(i, lens));
        let id = *ids.entry(activity.clone()).or_insert_with(|| {
            activities.push(activity);
            activities.len() - 1
        });
        activity_of.push(id);
        match fixed {
            Some(h) => confirmed.entry(h).or_default().push(i),
            None => strands.entry(strand.unwrap_or(0)).or_default().push(i),
        }
    }
    let mut app_ids: HashMap<&str, usize> = HashMap::new();
    for activity in &activities {
        let next = app_ids.len();
        app_ids.entry(&activity.app).or_insert(next);
    }

    // Partition each strand into groups of runs: one group per stretch of
    // work on one thing.
    let mut groups: Vec<Vec<Run>> = Vec::new();
    // Near misses: the span a cut would have fallen before, and an activity
    // from either side of it.
    let mut near: Vec<(usize, usize, usize)> = Vec::new();
    for strand in strands.values() {
        let strand_runs = runs(spans, strand, |i| activity_of[i], None);
        // An activity dropped for a long time and picked up again is, as far
        // as the pattern of switching goes, a new activity: what happened in
        // between was not an interruption of it.
        let mut bouts: HashMap<usize, (Time, usize)> = HashMap::new();
        let mut next_sym = 0;
        let visits: Vec<Visit> = strand_runs
            .iter()
            .map(|run| {
                let sym = match bouts.get(&run.activity) {
                    Some(&(end, sym)) if run.start.0 - end.0 <= params.max_gap => sym,
                    _ => {
                        next_sym += 1;
                        next_sym
                    }
                };
                bouts.insert(run.activity, (run.end, sym));
                Visit {
                    sym,
                    secs: run.end.0 - run.start.0,
                    app: app_ids[activities[run.activity].app.as_str()],
                }
            })
            .collect();
        let found = structure::segment(&visits, params.min_exclusive_share, params.near_miss_share);
        for group in &found {
            if let Some(miss) = &group.near_miss {
                let activity = |visit: usize| strand_runs[visit].activity;
                let at = strand_runs[miss.at].spans[0];
                near.push((at, activity(miss.before), activity(miss.after)));
            }
        }
        let mut strand_runs: Vec<Option<Run>> = strand_runs.into_iter().map(Some).collect();
        for group in found {
            let take = |i: usize| strand_runs[i].take().unwrap();
            groups.push(group.visits.into_iter().map(take).collect());
        }
    }
    groups.sort_by_key(|group| group[0].start);

    // Groups mostly spent on the same activity are the same task, resumed.
    let mut drafts: Vec<Draft> = Vec::new();
    let mut draft_of_main: HashMap<usize, usize> = HashMap::new();
    for group in groups {
        let mut secs: HashMap<usize, u64> = HashMap::new();
        for run in &group {
            *secs.entry(run.activity).or_default() += run.end.0 - run.start.0;
        }
        let total: u64 = secs.values().sum();
        let (&main, &most) = secs.iter().max_by_key(|&(&a, &s)| (s, a)).unwrap();
        let draft = if most * 2 >= total {
            *draft_of_main.entry(main).or_insert(drafts.len())
        } else {
            drafts.len()
        };
        if draft == drafts.len() {
            drafts.push(Draft::default());
        }
        drafts[draft].runs.extend(group);
    }
    for draft in &mut drafts {
        draft.runs.sort_by_key(|run| run.start);
    }

    // The user's changes to the result, one after another.
    for hint in hints {
        match hint {
            Hint::Split { at } => {
                // The span in focus at `at`, or the next if nothing was.
                let first = spans.partition_point(|s| s.end <= *at);
                let Some(from) = (first < spans.len())
                    .then(|| owner(&drafts, first))
                    .flatten()
                else {
                    continue;
                };
                let (mut before, mut after) = (Vec::new(), Vec::new());
                for run in std::mem::take(&mut drafts[from].runs) {
                    let (early, late): (Vec<usize>, Vec<usize>) =
                        run.spans.iter().partition(|&&i| spans[i].start < *at);
                    let part = |members: &[usize]| runs(spans, members, |_| run.activity, None);
                    before.extend(part(&early));
                    after.extend(part(&late));
                }
                drafts[from].runs = before;
                if drafts[from].runs.is_empty() {
                    drafts[from].runs = after;
                } else if !after.is_empty() {
                    drafts.push(Draft {
                        runs: after,
                        name: None,
                    });
                }
            }
            Hint::Join { from, into } => {
                let draft_at =
                    |t: Time| span_at(spans, t).and_then(|i| Some((i, owner(&drafts, i)?)));
                let (Some((span, from)), Some((_, into))) = (draft_at(*from), draft_at(*into))
                else {
                    continue;
                };
                if from == into {
                    continue;
                }
                let holds = |run: &usize| drafts[from].runs[*run].spans.contains(&span);
                let session = sessions_of(&drafts[from].runs, params.max_gap)
                    .into_iter()
                    .find(|session| session.iter().any(holds))
                    .unwrap();
                let (first, last) = (session[0], session[session.len() - 1]);
                let moved: Vec<Run> = drafts[from].runs.drain(first..=last).collect();
                drafts[into].runs.extend(moved);
                drafts[into].runs.sort_by_key(|run| run.start);
            }
            Hint::Combine { from, into } => {
                let draft_at = |t: Time| span_at(spans, t).and_then(|i| owner(&drafts, i));
                let (Some(from), Some(into)) = (draft_at(*from), draft_at(*into)) else {
                    continue;
                };
                if from == into {
                    continue;
                }
                // Each keeps its time as a part, named after what it was
                // mostly about (the user can rename it).
                for draft in [from, into] {
                    let label = part_label(&drafts[draft].runs, &activities, &titles);
                    for run in drafts[draft].runs.iter_mut().filter(|r| r.part.is_none()) {
                        run.part = Some(label.clone());
                    }
                }
                let moved = std::mem::take(&mut drafts[from].runs);
                drafts[into].runs.extend(moved);
                drafts[into].runs.sort_by_key(|run| run.start);
            }
            _ => {}
        }
    }
    // Names carried over from earlier days, then the user's own.
    for hint in hints {
        if let Hint::Usual { activity, name } = hint {
            for draft in drafts.iter_mut().filter(|d| d.name.is_none()) {
                if identity(&draft.runs, &activities).as_ref() == Some(activity) {
                    draft.name = Some(name.clone());
                }
            }
        }
    }
    for hint in hints {
        if let Hint::Name { at, name } = hint
            && let Some(draft) = span_at(spans, *at).and_then(|i| owner(&drafts, i))
        {
            drafts[draft].name = (!name.is_empty()).then(|| name.clone());
        }
    }
    for (h, members) in &confirmed {
        let Hint::Fixed { name, .. } = &hints[*h] else {
            unreachable!()
        };
        drafts.push(Draft {
            runs: runs(spans, members, |i| activity_of[i], Some(*h)),
            name: name.clone(),
        });
    }
    // Tasks given the same name are one task.
    let mut by_name: HashMap<String, usize> = HashMap::new();
    for i in 0..drafts.len() {
        let Some(name) = drafts[i].name.clone() else {
            continue;
        };
        let first = *by_name.entry(name).or_insert(i);
        if first != i {
            let moved = std::mem::take(&mut drafts[i].runs);
            drafts[first].runs.extend(moved);
            drafts[first].runs.sort_by_key(|run| run.start);
        }
    }
    // Divisions into parts: the user's latest for each task, or failing
    // that (and failing a combination), one carried over from earlier days.
    for draft in &mut drafts {
        let division = hints.iter().rev().find_map(|hint| match hint {
            Hint::Parts { at, parts }
                if span_at(spans, *at)
                    .is_some_and(|i| draft.runs.iter().any(|r| r.spans.contains(&i))) =>
            {
                Some(parts)
            }
            _ => None,
        });
        let division = division.or_else(|| {
            if draft.runs.iter().any(|r| r.part.is_some()) {
                return None;
            }
            let id = identity(&draft.runs, &activities)?;
            hints.iter().rev().find_map(|hint| match hint {
                Hint::UsualParts { activity, parts } if *activity == id => Some(parts),
                _ => None,
            })
        });
        if let Some(parts) = division {
            for run in &mut draft.runs {
                let window = &spans[run.spans[0]].window;
                run.part = parts
                    .iter()
                    .find(|(pattern, _)| pattern.matches(window))
                    .map(|(_, name)| name.clone());
            }
        }
    }
    drafts.retain(|draft| !draft.runs.is_empty());
    drafts.sort_by_key(|draft| draft.runs[0].start);

    // Put each task on the clock.
    let describe = Describe {
        spans,
        titles: &titles,
        activities: &activities,
        activity_of: &activity_of,
        near: &near,
        joined: hints
            .iter()
            .filter_map(|hint| match hint {
                Hint::Join { from, into } => Some((*from.min(into), *from.max(into))),
                _ => None,
            })
            .collect(),
    };
    let mut tasks: Vec<Task> = Vec::new();
    let mut sessions: Vec<Session> = Vec::new();
    for draft in drafts {
        let mut secs: HashMap<usize, u64> = HashMap::new();
        let mut all_spans = Vec::new();
        for run in &draft.runs {
            *secs.entry(run.activity).or_default() += run.end.0 - run.start.0;
            all_spans.extend(&run.spans);
        }
        let divided = draft.runs.iter().any(|r| r.part.is_some());
        for session in sessions_of(&draft.runs, params.max_gap) {
            let runs: Vec<&Run> = session.iter().map(|&i| &draft.runs[i]).collect();
            sessions.push(describe.session(tasks.len(), &runs, divided));
        }
        let mut by_time: Vec<(u64, usize)> = secs.into_iter().map(|(a, s)| (s, a)).collect();
        by_time.sort_by_key(|&(s, a)| (std::cmp::Reverse(s), a));
        tasks.push(Task {
            identity: identity(&draft.runs, &activities),
            precedent: None,
            parts: parts_of(draft.runs.iter(), divided),
            division: None,
            name: draft.name,
            label: describe.label(&all_spans),
            active: by_time.iter().map(|&(s, _)| s).sum(),
            activities: by_time
                .into_iter()
                .map(|(_, a)| activities[a].clone())
                .collect(),
            sessions: Vec::new(),
        });
    }

    sessions.sort_by_key(|s| s.start);
    let mut owner = vec![0; spans.len()];
    for (i, session) in sessions.iter().enumerate() {
        for &span in &session.spans {
            owner[span] = i;
        }
    }
    for i in 0..sessions.len() {
        let session = &sessions[i];
        let contains = |&p: &usize| {
            let parent = &sessions[p];
            p != i
                && parent.start <= session.start
                && session.end <= parent.end
                && parent.active >= session.active
        };
        // Carved out of what it interrupted, if that carried on afterwards;
        // failing that, out of the tightest session around it.
        let first = session.spans[0];
        let interrupted = first
            .checked_sub(1)
            .filter(|&before| spans[before].end == spans[first].start)
            .map(|before| owner[before]);
        let parent = interrupted.filter(contains).or_else(|| {
            (0..sessions.len())
                .filter(contains)
                .min_by_key(|&p| sessions[p].end.0 - sessions[p].start.0)
        });
        tasks[session.task].sessions.push(i);
        sessions[i].parent = parent;
    }

    // Doubts that only show once everything has its place: an activity with
    // an identity of its own that is also found in other tasks.
    let mut found_in: HashMap<&Activity, HashSet<usize>> = HashMap::new();
    for (t, task) in tasks.iter().enumerate() {
        for activity in &task.activities {
            found_in.entry(activity).or_default().insert(t);
        }
    }
    for session in &mut sessions {
        let main = &tasks[session.task].activities[0];
        for member in &mut session.members {
            if member.activity.subject.is_none() || member.activity == *main || session.confirmed {
                continue;
            }
            let others = found_in[&member.activity]
                .iter()
                .filter(|&&t| t != session.task);
            let own = others
                .clone()
                .any(|&t| tasks[t].activities[0] == member.activity);
            let tasks = others.count();
            if own || tasks >= 2 {
                member.doubts.push(Doubt::Elsewhere { tasks, own });
            }
        }
    }

    attach(&mut sessions, spans, &marks, params.mark_tolerance);

    Timeline {
        tracked: spans.iter().map(Span::secs).sum(),
        spans: spans.to_vec(),
        tasks,
        sessions,
        marks,
    }
}

/// Lists each of the user's markers with the session it belongs to: the
/// one starting at that moment, give or take `tolerance`; else the one in
/// focus at the time; else the next one, or failing that the last.
fn attach(sessions: &mut [Session], spans: &[Span], marks: &[Marker], tolerance: u64) {
    let mut owner = vec![0; spans.len()];
    for (i, session) in sessions.iter().enumerate() {
        for &span in &session.spans {
            owner[span] = i;
        }
    }
    for (k, marker) in marks.iter().enumerate() {
        if !marker.mark.by_user() {
            continue;
        }
        let t = marker.at;
        let starting = (0..sessions.len())
            .filter(|&i| sessions[i].start.0.abs_diff(t.0) <= tolerance)
            .min_by_key(|&i| (sessions[i].start.0.abs_diff(t.0), std::cmp::Reverse(i)));
        if let Some(i) = starting
            && sessions[i].opened_by.is_none()
        {
            sessions[i].opened_by = Some(k);
            continue;
        }
        let next = spans.partition_point(|s| s.end <= t);
        let span = match (next < spans.len(), spans.len()) {
            (true, _) => next,
            (false, 0) => continue,
            (false, n) => n - 1,
        };
        sessions[owner[span]].marks.push(k);
    }
}

/// What is needed to describe a set of runs to the user.
struct Describe<'a> {
    spans: &'a [Span],
    titles: &'a Titles,
    activities: &'a [Activity],
    activity_of: &'a [usize],
    near: &'a [(usize, usize, usize)],
    /// Pairs of moments the user has said belong to the same task.
    joined: Vec<(Time, Time)>,
}

impl Describe<'_> {
    /// An activity, as the user would recognise it.
    fn name(&self, activity: usize) -> String {
        let Activity { app, subject } = &self.activities[activity];
        let app = self.titles.display_name(app);
        match subject {
            Some(subject) => format!("{app} · {subject}"),
            None => app.to_string(),
        }
    }

    /// An activity, by what sets it apart from others in the same app.
    fn short(&self, activity: usize) -> String {
        let Activity { app, subject } = &self.activities[activity];
        let app = || self.titles.display_name(app).to_string();
        subject.clone().unwrap_or_else(app)
    }

    /// One session of task `task`, made of `runs`; `divided` if the task is
    /// divided into parts.
    fn session(&self, task: usize, runs: &[&Run], divided: bool) -> Session {
        let confirmed = runs.iter().all(|run| run.fixed.is_some());
        let spans: Vec<usize> = runs.iter().flat_map(|run| run.spans.clone()).collect();
        let active: u64 = spans.iter().map(|&i| self.spans[i].secs()).sum();
        let (start, end) = (runs[0].start, runs[runs.len() - 1].end);

        // What it is made of.
        let mut members: Vec<Member> = Vec::new();
        let mut index: HashMap<usize, usize> = HashMap::new();
        let mut app_visits: HashMap<&str, usize> = HashMap::new();
        for run in runs {
            let activity = &self.activities[run.activity];
            *app_visits.entry(&activity.app).or_default() += 1;
            let m = *index.entry(run.activity).or_insert_with(|| {
                members.push(Member {
                    activity: activity.clone(),
                    name: self.name(run.activity),
                    secs: 0,
                    visits: 0,
                    why: Why::Once,
                    doubts: Vec::new(),
                    spans: Vec::new(),
                    precedent: None,
                    part: None,
                });
                members.len() - 1
            });
            members[m].secs += run.end.0 - run.start.0;
            members[m].visits += 1;
            members[m].spans.extend(&run.spans);
            if run.part.is_some() {
                members[m].part = run.part.clone();
            }
        }
        members.sort_by(|a, b| (b.secs, &a.activity).cmp(&(a.secs, &b.activity)));
        for (m, member) in members.iter_mut().enumerate() {
            member.why = match () {
                _ if confirmed => Why::Confirmed,
                _ if m == 0 => Why::Main,
                _ if member.visits >= 2 => Why::Returned,
                _ if app_visits[member.activity.app.as_str()] >= 2 => Why::SameApp,
                _ => Why::Once,
            };
        }

        // Where it might be split.
        let mut splits: Vec<PossibleSplit> = Vec::new();
        if !confirmed {
            // Where the guess nearly split it.
            for &(span, before, after) in self.near {
                if spans.contains(&span) && self.spans[span].start > start {
                    splits.push(PossibleSplit {
                        at: self.spans[span].start,
                        before: self.short(before),
                        after: self.short(after),
                    });
                }
            }
            // Where an app that matters here moves for good from one main
            // subject to another, however much else carries on across it.
            let mut by_app: HashMap<&str, Vec<&Run>> = HashMap::new();
            for &run in runs {
                let app = self.activities[run.activity].app.as_str();
                by_app.entry(app).or_default().push(run);
            }
            for runs in by_app.values() {
                let secs = |runs: &[&Run]| runs.iter().map(|r| r.end.0 - r.start.0).sum::<u64>();
                let total = secs(runs);
                if total * 10 < active {
                    continue;
                }
                let mut last: HashMap<usize, usize> = HashMap::new();
                for (i, run) in runs.iter().enumerate() {
                    last.insert(run.activity, i);
                }
                // The subject most of a side's time went into, if one did.
                let main = |side: &[&Run]| {
                    let mut time: HashMap<usize, u64> = HashMap::new();
                    for run in side {
                        *time.entry(run.activity).or_default() += run.end.0 - run.start.0;
                    }
                    let (&activity, &most) = time.iter().max_by_key(|&(&a, &s)| (s, a))?;
                    (most * 2 > secs(side) && secs(side) * 4 >= total).then_some(activity)
                };
                let mut reach = 0;
                for i in 0..runs.len() - 1 {
                    reach = reach.max(last[&runs[i].activity]);
                    let (left, right) = runs.split_at(i + 1);
                    if reach == i
                        && let (Some(before), Some(after)) = (main(left), main(right))
                    {
                        splits.push(PossibleSplit {
                            at: right[0].start,
                            before: self.short(before),
                            after: self.short(after),
                        });
                    }
                }
            }
            // Not between two moments the user has said belong together.
            let settled = |at: Time| {
                let around = |&(a, b): &(Time, Time)| start <= a && a < at && at <= b && b < end;
                self.joined.iter().any(around)
            };
            splits.retain(|split| !settled(split.at));
            splits.sort_by_key(|split| split.at);
            splits.dedup_by_key(|split| split.at);
        }

        Session {
            task,
            start,
            end,
            active,
            visits: runs.len(),
            parent: None,
            label: self.label(&spans),
            members,
            splits,
            confirmed,
            opened_by: None,
            marks: Vec::new(),
            parts: parts_of(runs.iter().copied(), divided),
            spans,
        }
    }

    /// Describes a set of spans by what their titles dwelt on.
    fn label(&self, members: &[usize]) -> Label {
        let (spans, titles) = (self.spans, self.titles);
        // Per title part: whether it is only a detail of something broader
        // (a file of a project), seconds, and how close to the app's name it
        // sits (the nearer, the broader).
        let mut parts: HashMap<&str, (bool, u64, usize)> = HashMap::new();
        let mut apps: HashMap<&str, u64> = HashMap::new();
        for &i in members {
            let secs = spans[i].secs();
            *apps.entry(titles.stem(i)).or_default() += secs;
            let subject = titles.subject(i);
            let identity = &self.activities[self.activity_of[i]].subject;
            for (pos, part) in subject.iter().enumerate() {
                if title::is_informative(part) {
                    let detail = identity.as_ref().is_some_and(|id| id != part);
                    let entry = parts.entry(part).or_insert((true, 0, usize::MAX));
                    entry.0 &= detail;
                    entry.1 += secs;
                    entry.2 = entry.2.min(subject.len() - pos);
                }
            }
        }
        let mut apps: Vec<(&str, u64)> = apps.into_iter().collect();
        apps.sort_by_key(|&(app, secs)| (std::cmp::Reverse(secs), app));
        let apps: Vec<String> = apps
            .into_iter()
            .map(|(app, _)| titles.display_name(app).to_string())
            .collect();
        let mut parts: Vec<(&str, (bool, u64, usize))> = parts.into_iter().collect();
        parts.sort_by_key(|&(part, (_, secs, depth))| (std::cmp::Reverse(secs), depth, part));
        // The headline is the most dwelt-on part that isn't a detail.
        let headline = parts.iter().position(|&(_, (detail, ..))| !detail);
        let headline = headline.map(|i| parts.remove(i).0.to_string());
        let also = parts
            .into_iter()
            .take(3)
            .map(|(part, (_, secs, _))| Evidence {
                text: part.to_string(),
                secs,
            });
        Label {
            headline: headline.unwrap_or_else(|| apps.join(" + ")),
            also: also.collect(),
            apps,
        }
    }
}
