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
//! The grouping is a guess. [`Hint`]s are how a user (through a UI that does
//! not exist yet) corrects it; [`interpret`] is a pure function of the log
//! and the hints, so applying a correction means running it again.

pub mod model;
pub mod render;
pub mod scenarios;
mod structure;
pub mod title;

use std::collections::HashMap;

pub use model::{Event, Span, Time, Window};
pub use title::{Activity, Lens, Pattern};

use structure::Visit;
use title::Titles;

/// A correction to the guessed interpretation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hint {
    /// How much of this app's titles counts towards identity.
    Lens { app: String, lens: Lens },
    /// These windows are a task of their own, whatever they alternate with.
    Apart(Pattern),
    /// Windows matching either pattern are the same activity, so they are
    /// never the reason to tell two tasks apart.
    Together(Pattern, Pattern),
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
}

impl Default for Params {
    fn default() -> Params {
        Params {
            max_gap: 15 * 60,
            min_exclusive_share: 0.5,
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

/// Something the user was working on.
#[derive(Clone, Debug)]
pub struct Task {
    pub label: Label,
    /// The activities that make it up, most time first.
    pub activities: Vec<Activity>,
    /// Seconds spent on it, over all sessions.
    pub active: u64,
    /// Indexes into [`Timeline::sessions`].
    pub sessions: Vec<usize>,
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
    /// Indexes into the log's spans.
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
#[derive(Clone, Debug)]
pub struct Timeline {
    /// Ordered by first appearance.
    pub tasks: Vec<Task>,
    /// Ordered by start.
    pub sessions: Vec<Session>,
    /// Seconds of the log during which some window had focus. Always the sum
    /// of the tasks' (and of the sessions') active time.
    pub tracked: u64,
}

struct Run {
    activity: usize,
    start: Time,
    end: Time,
    spans: Vec<usize>,
}

impl Run {
    fn secs(&self, spans: &[Span]) -> u64 {
        self.spans.iter().map(|&i| spans[i].secs()).sum()
    }
}

/// Interprets a focus log. `spans` must be in time order and not overlap,
/// as produced by [`model::spans`].
pub fn interpret(spans: &[Span], hints: &[Hint], params: &Params) -> Timeline {
    let titles = Titles::learn(spans);

    // What each span is, and which strand it is analysed in: strand 0 is
    // everything the hints don't set apart.
    let mut activities: Vec<Activity> = Vec::new();
    let mut ids: HashMap<Activity, usize> = HashMap::new();
    let mut strands: Vec<Vec<usize>> = vec![Vec::new()];
    let mut strand_of: HashMap<usize, usize> = HashMap::new();
    let mut activity_of = Vec::with_capacity(spans.len());
    for (i, span) in spans.iter().enumerate() {
        let window = &span.window;
        let mut activity = None;
        let mut lens = Lens::Auto;
        let mut strand = 0;
        for (h, hint) in hints.iter().enumerate() {
            match hint {
                Hint::Lens { app, lens: l } if title::app_stem(app) == titles.stem(i) => lens = *l,
                Hint::Apart(pattern) if strand == 0 && pattern.matches(window) => {
                    let next = strands.len();
                    strand = *strand_of.entry(h).or_insert(next);
                    if strand == next {
                        strands.push(Vec::new());
                    }
                }
                Hint::Together(a, b)
                    if activity.is_none() && (a.matches(window) || b.matches(window)) =>
                {
                    activity = Some(Activity {
                        app: format!("{a} + {b}"),
                        subject: None,
                    });
                }
                _ => {}
            }
        }
        let activity = activity.unwrap_or_else(|| titles.activity(i, lens));
        let id = *ids.entry(activity.clone()).or_insert_with(|| {
            activities.push(activity);
            activities.len() - 1
        });
        activity_of.push(id);
        strands[strand].push(i);
    }
    let mut app_ids: HashMap<&str, usize> = HashMap::new();
    for activity in &activities {
        let next = app_ids.len();
        app_ids.entry(&activity.app).or_insert(next);
    }

    // Partition each strand into groups of runs: one group per stretch of
    // work on one thing.
    let mut groups: Vec<Vec<Run>> = Vec::new();
    for strand in &strands {
        let mut runs: Vec<Run> = Vec::new();
        for &i in strand {
            match runs.last_mut() {
                Some(run) if run.activity == activity_of[i] && run.end == spans[i].start => {
                    run.end = spans[i].end;
                    run.spans.push(i);
                }
                _ => runs.push(Run {
                    activity: activity_of[i],
                    start: spans[i].start,
                    end: spans[i].end,
                    spans: vec![i],
                }),
            }
        }
        // An activity dropped for a long time and picked up again is, as far
        // as the pattern of switching goes, a new activity: what happened in
        // between was not an interruption of it.
        let mut bouts: HashMap<usize, (Time, usize)> = HashMap::new();
        let mut next_sym = 0;
        let visits: Vec<Visit> = runs
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
                let activity = &activities[run.activity];
                Visit {
                    sym,
                    secs: run.secs(spans),
                    app: app_ids[activity.app.as_str()],
                }
            })
            .collect();
        let mut runs: Vec<Option<Run>> = runs.into_iter().map(Some).collect();
        for group in structure::segment(&visits, params.min_exclusive_share) {
            groups.push(group.into_iter().map(|i| runs[i].take().unwrap()).collect());
        }
    }
    groups.sort_by_key(|group| group[0].start);

    // Groups mostly spent on the same activity are the same task, resumed.
    let mut task_runs: Vec<Vec<Run>> = Vec::new();
    let mut task_of_main: HashMap<usize, usize> = HashMap::new();
    for group in groups {
        let mut secs: HashMap<usize, u64> = HashMap::new();
        for run in &group {
            *secs.entry(run.activity).or_default() += run.secs(spans);
        }
        let total: u64 = secs.values().sum();
        let (&main, &most) = secs.iter().max_by_key(|&(&a, &s)| (s, a)).unwrap();
        let task = if most * 2 >= total {
            *task_of_main.entry(main).or_insert(task_runs.len())
        } else {
            task_runs.len()
        };
        if task == task_runs.len() {
            task_runs.push(Vec::new());
        }
        task_runs[task].extend(group);
    }

    // Put each task on the clock.
    let mut tasks: Vec<Task> = Vec::new();
    let mut sessions: Vec<Session> = Vec::new();
    for mut runs in task_runs {
        runs.sort_by_key(|run| run.start);
        let active: u64 = runs.iter().map(|run| run.secs(spans)).sum();
        let mut secs: HashMap<usize, u64> = HashMap::new();
        let mut all_spans = Vec::new();
        let mut open: Option<Session> = None;
        for run in &runs {
            *secs.entry(run.activity).or_default() += run.secs(spans);
            all_spans.extend(&run.spans);
            let resumes = |s: &Session| run.start.0 - s.end.0 <= params.max_gap;
            match &mut open {
                Some(session) if resumes(session) => {
                    session.end = run.end;
                    session.active += run.secs(spans);
                    session.visits += 1;
                    session.spans.extend(&run.spans);
                }
                _ => {
                    sessions.extend(open.take());
                    open = Some(Session {
                        task: tasks.len(),
                        start: run.start,
                        end: run.end,
                        active: run.secs(spans),
                        visits: 1,
                        parent: None,
                        label: label(spans, &titles, &run.spans),
                        spans: run.spans.clone(),
                    });
                }
            }
        }
        sessions.extend(open);
        let mut by_time: Vec<(u64, usize)> = secs.into_iter().map(|(a, s)| (s, a)).collect();
        by_time.sort_by_key(|&(s, a)| (std::cmp::Reverse(s), a));
        tasks.push(Task {
            label: label(spans, &titles, &all_spans),
            activities: by_time
                .into_iter()
                .map(|(_, a)| activities[a].clone())
                .collect(),
            active,
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
        let label = label(spans, &titles, &session.spans);
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
        sessions[i].label = label;
        sessions[i].parent = parent;
    }

    Timeline {
        tracked: spans.iter().map(Span::secs).sum(),
        tasks,
        sessions,
    }
}

/// Describes a set of spans by what their titles dwelt on.
fn label(spans: &[Span], titles: &Titles, members: &[usize]) -> Label {
    // Per title part: seconds, and how close to the app's name it sits (the
    // nearer, the broader: a project rather than a file).
    let mut parts: HashMap<&str, (u64, usize)> = HashMap::new();
    let mut apps: HashMap<&str, u64> = HashMap::new();
    for &i in members {
        let secs = spans[i].secs();
        *apps.entry(titles.stem(i)).or_default() += secs;
        let subject = titles.subject(i);
        for (pos, part) in subject.iter().enumerate() {
            if title::is_informative(part) {
                let entry = parts.entry(part).or_insert((0, usize::MAX));
                entry.0 += secs;
                entry.1 = entry.1.min(subject.len() - pos);
            }
        }
    }
    let mut apps: Vec<(&str, u64)> = apps.into_iter().collect();
    apps.sort_by_key(|&(app, secs)| (std::cmp::Reverse(secs), app));
    let apps: Vec<String> = apps
        .into_iter()
        .map(|(app, _)| titles.display_name(app).to_string())
        .collect();
    let mut parts: Vec<(&str, (u64, usize))> = parts.into_iter().collect();
    parts.sort_by_key(|&(part, (secs, depth))| (std::cmp::Reverse(secs), depth, part));
    let mut parts = parts.into_iter().map(|(part, (secs, _))| Evidence {
        text: part.to_string(),
        secs,
    });
    Label {
        headline: parts.next().map_or_else(|| apps.join(" + "), |e| e.text),
        apps,
        also: parts.take(3).collect(),
    }
}
