//! What earlier days teach: rules carried over from the corrections the user
//! made then.
//!
//! Corrections are made at times of day, which mean nothing the next day,
//! but they are about windows, which come back. Re-read with its own
//! corrections, a past day says what they were about:
//!
//! - **Names.** A task the user named was mostly about some activity (its
//!   [`Task::identity`]: the project, the site). Work on that activity
//!   another day is probably called the same.
//! - **Separations.** A window the user took out of a row (just there, or
//!   everywhere) is a thing of its own, done on the side: mail, chat.
//!
//! Like a marker, a carried-over rule is evidence, not an order, and how
//! much it counts depends on how consistently it held:
//!
//! - After one day it is only *offered*: shown on the row, for the user to
//!   take or leave. A name that changes from day to day (a ticket number)
//!   is offered only once it has held two days running.
//! - After [`APPLY_AFTER`] days in a row without anything against it, it is
//!   *applied*, as a standing hint that comes before the day's own
//!   corrections, so that anything the user does today overrides it. A
//!   window the user said was a task of its own *everywhere* is applied
//!   from the next day.
//! - Renaming the task, taking its name away, or joining a carved-out window
//!   back counts against the rule: it has to start again from there.
//!
//! Days are read in order, each with the rules learned from the days before
//! it, as it was seen at the time: that is what makes a correction against
//! an applied rule visible as one.

use std::collections::{BTreeMap, BTreeSet};

use super::journal::Journal;
use super::model::{Marker, Record, Span, Time};
use super::title::{Activity, Pattern};
use super::{Hint, Params, Timeline, interpret_marked};

/// How many days in a row a rule must hold to be applied without asking.
pub const APPLY_AFTER: usize = 3;
/// How many days back a rule's record goes.
const MEMORY: usize = 10;

/// One earlier day: its log, markers and the corrections left standing.
#[derive(Clone, Debug, Default)]
pub struct Day {
    /// What to call the day when saying a rule comes from it ("Mon",
    /// "2026-11-09").
    pub label: String,
    pub spans: Vec<Span>,
    pub marks: Vec<Marker>,
    pub hints: Vec<Hint>,
}

impl Day {
    /// A day from the lines of its log.
    pub fn from_records(label: &str, records: &[Record]) -> Day {
        let mut journal = Journal::default();
        for record in records {
            journal.record(record.clone());
        }
        let end = journal.latest().unwrap_or(Time(0));
        Day {
            label: label.to_string(),
            spans: journal.spans(end),
            marks: journal.marks().to_vec(),
            hints: journal.hints().to_vec(),
        }
    }
}

/// What earlier days say about a task's name, or about one of the things
/// swept into a row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Precedent {
    /// For a task: the name it went by.
    pub name: Option<String>,
    /// The days it held, oldest first (labels of [`Day`]s).
    pub days: Vec<String>,
    /// Whether it is applied today without asking (otherwise it is only
    /// offered).
    pub applied: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Naming {
    Named(String),
    /// A name carried over was taken away.
    Cleared,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Parting {
    /// Taken out of the row it was in, there.
    TakenOut,
    /// Said to be a task of its own wherever it shows up.
    Always,
    /// Carved out by a carried-over rule, and joined back by the user.
    JoinedBack,
}

/// Rules learned from earlier days.
#[derive(Clone, Debug, Default)]
pub struct Lessons {
    names: BTreeMap<Activity, Vec<(String, Naming)>>,
    parted: BTreeMap<Activity, Vec<(String, Parting)>>,
}

impl Lessons {
    /// Learns from these days, given oldest first.
    pub fn learn(days: &[Day], params: &Params) -> Lessons {
        let mut lessons = Lessons::default();
        for day in days {
            lessons.observe(day, params);
        }
        lessons
    }

    /// The rules applied without asking, as hints to put before the day's
    /// own.
    pub fn standing(&self) -> Vec<Hint> {
        let mut out = Vec::new();
        for activity in self.parted.keys() {
            if self.parting(activity).is_some_and(|p| p.applied) {
                out.push(Hint::apart(pattern_of(activity)));
            }
        }
        for activity in self.names.keys() {
            if let Some(Precedent {
                name: Some(name),
                applied: true,
                ..
            }) = self.naming(activity)
            {
                out.push(Hint::Usual {
                    activity: activity.clone(),
                    name,
                });
            }
        }
        out
    }

    /// What earlier days say this activity's work is called, if anything.
    pub fn naming(&self, activity: &Activity) -> Option<Precedent> {
        let events = self.names.get(activity)?;
        let latest = match events.last()? {
            (_, Naming::Named(name)) => name,
            (_, Naming::Cleared) => return None,
        };
        let mut days: Vec<String> = events
            .iter()
            .rev()
            .take_while(|(_, e)| *e == Naming::Named(latest.clone()))
            .map(|(day, _)| day.clone())
            .collect();
        days.dedup();
        days.reverse();
        let names: BTreeSet<&Naming> = events.iter().map(|(_, e)| e).collect();
        let varied = names.len() > 1;
        if varied && days.len() < 2 {
            return None;
        }
        Some(Precedent {
            name: Some(latest.clone()),
            applied: days.len() >= APPLY_AFTER,
            days,
        })
    }

    /// Whether earlier days say this activity is a task of its own.
    pub fn parting(&self, activity: &Activity) -> Option<Precedent> {
        let events = self.parted.get(activity)?;
        let streak: Vec<&(String, Parting)> = events
            .iter()
            .rev()
            .take_while(|(_, e)| *e != Parting::JoinedBack)
            .collect();
        let mut days: Vec<String> = streak.iter().rev().map(|(d, _)| d.clone()).collect();
        days.dedup();
        if days.is_empty() {
            return None;
        }
        let always = streak.iter().any(|(_, e)| *e == Parting::Always);
        Some(Precedent {
            name: None,
            applied: always || days.len() >= APPLY_AFTER,
            days,
        })
    }

    /// Notes on today's timeline what earlier days say about its tasks and
    /// about what was swept into its rows.
    pub fn annotate(&self, timeline: &mut Timeline) {
        for task in &mut timeline.tasks {
            task.precedent = task.identity.as_ref().and_then(|a| self.naming(a));
        }
        // A separation is worth saying only where it matters: offered for
        // something swept into a row with other things, and, when applied,
        // on the row it carved out.
        for session in &mut timeline.sessions {
            let (shared, carved) = (session.members.len() > 1, session.parent.is_some());
            for member in &mut session.members {
                member.precedent = self
                    .parting(&member.activity)
                    .filter(|p| if p.applied { carved } else { shared });
            }
        }
    }

    /// Reads one more day, as it was seen with the rules learned so far.
    fn observe(&mut self, day: &Day, params: &Params) {
        let mut hints = self.standing();
        hints.extend(day.hints.iter().cloned());
        let t = interpret_marked(&day.spans, &day.marks, &hints, params);
        let label = &day.label;

        // What each task was called, by what it was about.
        let mut seen: BTreeMap<Activity, BTreeSet<String>> = BTreeMap::new();
        let mut cleared: BTreeSet<Activity> = BTreeSet::new();
        for task in &t.tasks {
            let Some(activity) = &task.identity else {
                continue;
            };
            match &task.name {
                Some(name) => {
                    seen.entry(activity.clone())
                        .or_default()
                        .insert(name.clone());
                }
                // Left without the name a rule would have given it: the user
                // took it away.
                None if self.naming(activity).is_some_and(|p| p.applied) => {
                    cleared.insert(activity.clone());
                }
                None => {}
            }
        }
        for (activity, names) in seen {
            let events = self.names.entry(activity).or_default();
            events.extend(names.into_iter().map(|n| (label.clone(), Naming::Named(n))));
        }
        for activity in cleared {
            let events = self.names.entry(activity).or_default();
            events.push((label.clone(), Naming::Cleared));
        }

        // What the user set apart, and what they joined back.
        let mut activity_of: Vec<Option<&Activity>> = vec![None; t.spans.len()];
        for session in &t.sessions {
            for member in &session.members {
                for &span in &member.spans {
                    activity_of[span] = Some(&member.activity);
                }
            }
        }
        let mut parted: BTreeMap<Activity, Parting> = BTreeMap::new();
        for hint in &day.hints {
            match hint {
                Hint::Apart { pattern, during } => {
                    for (i, span) in t.spans.iter().enumerate() {
                        let inside = during.is_empty()
                            || during
                                .iter()
                                .any(|&(a, b)| a <= span.start && span.start < b);
                        if let Some(activity) = activity_of[i]
                            && inside
                            && pattern.matches(&span.window)
                        {
                            let event = match during.is_empty() {
                                true => Parting::Always,
                                false => Parting::TakenOut,
                            };
                            let entry = parted.entry(activity.clone()).or_insert(event);
                            if event == Parting::Always {
                                *entry = event;
                            }
                        }
                    }
                }
                Hint::Join { from, .. } => {
                    let i = t.spans.partition_point(|s| s.end <= *from);
                    if let Some(activity) = activity_of.get(i).copied().flatten()
                        && self.parting(activity).is_some_and(|p| p.applied)
                    {
                        parted.insert(activity.clone(), Parting::JoinedBack);
                    }
                }
                _ => {}
            }
        }
        for (activity, event) in parted {
            self.parted
                .entry(activity)
                .or_default()
                .push((label.clone(), event));
        }

        for events in self.names.values_mut() {
            forget(events);
        }
        for events in self.parted.values_mut() {
            forget(events);
        }
    }
}

/// Keeps only what happened on the last [`MEMORY`] days with anything.
fn forget<T>(events: &mut Vec<(String, T)>) {
    let mut days: Vec<&String> = events.iter().map(|(d, _)| d).collect();
    days.dedup();
    if days.len() > MEMORY {
        let first = days[days.len() - MEMORY].clone();
        let keep = events.iter().position(|(d, _)| *d == first).unwrap_or(0);
        events.drain(..keep);
    }
}

/// Days, for saying where a rule comes from: "Mon", "Mon, Tue",
/// "Mon to Fri (5 days)".
pub fn days(days: &[String]) -> String {
    match days {
        [] => String::new(),
        [one] => one.clone(),
        [a, b] => format!("{a}, {b}"),
        [a, b, c] => format!("{a}, {b}, {c}"),
        [first, .., last] => format!("{first} to {last} ({} days)", days.len()),
    }
}

/// The windows an activity stands for.
fn pattern_of(activity: &Activity) -> Pattern {
    Pattern {
        app: Some(activity.app.clone()),
        title: activity.subject.as_ref().map(|s| s.to_lowercase()),
    }
}
