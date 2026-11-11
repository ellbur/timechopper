//! A focus log that is still being written, the user's corrections to its
//! interpretation, and the operations a UI offers on the rows it shows.
//!
//! The intended loop: the watcher calls [`Journal::observe`] on every focus
//! change; the UI calls [`Journal::timeline`] whenever it wants to redraw,
//! which interprets everything up to the present moment. When the user acts
//! on a row, the UI asks the timeline it is showing for the corresponding
//! hint ([`Timeline::split`], [`Timeline::join`], [`Timeline::separate`],
//! [`Timeline::name`], [`Timeline::confirm`]), may show what it would do
//! ([`Journal::preview`]), and applies it ([`Journal::apply`]).

use super::history::Lessons;
use super::model::{Marker, Record, Span, Time, Window};
use super::{Hint, Params, Pattern, Session, Timeline, interpret_marked};

/// A log, as far as it has got, and what the user has said about it.
#[derive(Clone, Debug, Default)]
pub struct Journal {
    spans: Vec<Span>,
    /// The window in focus now, and since when.
    open: Option<(Time, Window)>,
    marks: Vec<Marker>,
    hints: Vec<Hint>,
    /// What earlier days teach, applied before the day's own corrections.
    lessons: Lessons,
    pub params: Params,
}

impl Journal {
    pub fn new(params: Params) -> Journal {
        Journal {
            params,
            ..Journal::default()
        }
    }

    /// A journal of a log that is already complete.
    pub fn from_spans(spans: Vec<Span>, params: Params) -> Journal {
        Journal {
            spans,
            params,
            ..Journal::default()
        }
    }

    /// Focus moved to `window` at `at`, or to nothing: the user went idle.
    /// Calls must be in time order.
    pub fn observe(&mut self, at: Time, window: Option<Window>) {
        if self
            .open
            .as_ref()
            .is_some_and(|(_, w)| Some(w) == window.as_ref())
        {
            return;
        }
        if let Some((start, window)) = self.open.take() {
            self.close(start, at, window);
        }
        self.open = window.map(|w| (at, w));
    }

    /// Carries over what earlier days teach (see [`super::history`]).
    pub fn learn(&mut self, lessons: Lessons) {
        self.lessons = lessons;
    }

    pub fn lessons(&self) -> &Lessons {
        &self.lessons
    }

    /// Something was marked: by the user, or a window opening or closing.
    pub fn mark(&mut self, marker: Marker) {
        let i = self.marks.partition_point(|m| m.at <= marker.at);
        self.marks.insert(i, marker);
    }

    /// Takes back the user's (latest) marker made at `at`.
    pub fn unmark(&mut self, at: Time) -> Option<Marker> {
        let i = self
            .marks
            .iter()
            .rposition(|m| m.at == at && m.mark.by_user())?;
        Some(self.marks.remove(i))
    }

    /// Takes in one line of a log.
    pub fn record(&mut self, record: Record) {
        match record {
            Record::Focus(event) => self.observe(event.at, event.window),
            Record::Mark(marker) => self.mark(marker),
            Record::Unmark { mark, .. } => {
                self.unmark(mark);
            }
            Record::Hint { hint, .. } => self.apply(hint),
            Record::Unhint { .. } => {
                self.undo();
            }
        }
    }

    pub fn marks(&self) -> &[Marker] {
        &self.marks
    }

    fn close(&mut self, start: Time, end: Time, window: Window) {
        if end <= start {
            return;
        }
        match self.spans.last_mut() {
            Some(prev) if prev.end == start && prev.window == window => prev.end = end,
            _ => self.spans.push(Span { start, end, window }),
        }
    }

    /// The time of the latest observation, if there has been one.
    pub fn latest(&self) -> Option<Time> {
        let last = self.spans.last().map(|s| s.end);
        last.max(self.open.as_ref().map(|(at, _)| *at))
    }

    /// The log up to `now`, including the window in focus at that moment.
    pub fn spans(&self, now: Time) -> Vec<Span> {
        let mut log = self.clone();
        log.observe(now, None);
        log.spans
    }

    /// The interpretation of everything up to `now`.
    pub fn timeline(&self, now: Time) -> Timeline {
        let marks: Vec<Marker> = self.marks.iter().filter(|m| m.at <= now).cloned().collect();
        let mut hints = self.lessons.standing();
        hints.extend(self.hints.iter().cloned());
        let mut timeline = interpret_marked(&self.spans(now), &marks, &hints, &self.params);
        self.lessons.annotate(&mut timeline);
        timeline
    }

    pub fn hints(&self) -> &[Hint] {
        &self.hints
    }

    /// Adds a correction. It applies from the next [`Journal::timeline`] on.
    pub fn apply(&mut self, hint: Hint) {
        self.hints.push(hint);
    }

    /// Takes back the latest correction.
    pub fn undo(&mut self) -> Option<Hint> {
        self.hints.pop()
    }

    /// Takes back one correction, by its position in [`Journal::hints`].
    pub fn retract(&mut self, index: usize) -> Hint {
        self.hints.remove(index)
    }

    /// What the timeline would be with one more correction, and what that
    /// would change, without applying it.
    pub fn preview(&self, hint: Hint, now: Time) -> (Timeline, Diff) {
        let mut with = self.clone();
        with.apply(hint);
        let (before, after) = (self.timeline(now), with.timeline(now));
        let diff = before.diff(&after);
        (after, diff)
    }
}

/// How far a correction made on one row reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Just what is in this row.
    Here,
    /// Every window like it, in the whole log.
    Everywhere,
}

/// The rows that differ between two timelines: everything else is the same
/// stretch of time, counted the same way, under the same name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    /// Sessions of the first timeline that the second doesn't have.
    pub gone: Vec<usize>,
    /// Sessions of the second timeline that the first didn't have.
    pub new: Vec<usize>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.gone.is_empty() && self.new.is_empty()
    }
}

/// Operations on rows. Each returns the hint that would carry it out; none
/// changes anything by itself. `session` and `member` are indexes into this
/// timeline's sessions and into that session's members.
impl Timeline {
    /// The stretches of time during which a session had focus.
    pub fn intervals(&self, session: usize) -> Vec<(Time, Time)> {
        self.merge(&self.sessions[session].spans)
    }

    fn merge(&self, spans: &[usize]) -> Vec<(Time, Time)> {
        let mut out: Vec<(Time, Time)> = Vec::new();
        for &i in spans {
            let span = &self.spans[i];
            match out.last_mut() {
                Some(last) if last.1 == span.start => last.1 = span.end,
                _ => out.push((span.start, span.end)),
            }
        }
        out
    }

    /// The start of the session's first time in focus: a moment that
    /// identifies the session whatever else changes around it.
    pub fn anchor(&self, session: usize) -> Time {
        self.spans[self.sessions[session].spans[0]].start
    }

    /// From `at` on, this session (and whatever its task does later) is a
    /// different task. Also how a [`super::PossibleSplit`] is accepted.
    /// `None` if that leaves nothing on one side.
    pub fn split(&self, session: usize, at: Time) -> Option<Hint> {
        // A session only changes hands while it has focus: if it didn't at
        // `at`, the split takes effect when it next does.
        let spans = self.sessions[session].spans.iter();
        let mut after = spans.map(|&i| &self.spans[i]).filter(|s| s.end > at);
        let at = at.max(after.next()?.start);
        (at > self.sessions[session].start).then_some(Hint::Split { at })
    }

    /// This session is part of the task that session `into` belongs to: two
    /// rows the guess kept apart are the same thing, or a carve-out was no
    /// interruption after all.
    pub fn join(&self, session: usize, into: usize) -> Option<Hint> {
        (self.sessions[session].task != self.sessions[into].task).then(|| Hint::Join {
            from: self.anchor(session),
            into: self.anchor(into),
        })
    }

    /// One of the things swept into this session is a task of its own:
    /// here, or wherever it shows up.
    pub fn separate(&self, session: usize, member: usize, scope: Scope) -> Hint {
        let member = &self.sessions[session].members[member];
        let app = Pattern::app(&member.activity.app);
        match (scope, &member.activity.subject) {
            (Scope::Here, _) => Hint::Apart {
                pattern: app,
                during: self.merge(&member.spans),
            },
            (Scope::Everywhere, Some(subject)) => Hint::apart(Pattern {
                title: Some(subject.to_lowercase()),
                ..app
            }),
            (Scope::Everywhere, None) => Hint::apart(app),
        }
    }

    /// What the user calls this session's task.
    pub fn name(&self, session: usize, name: &str) -> Hint {
        Hint::Name {
            at: self.anchor(session),
            name: name.to_string(),
        }
    }

    /// This row is right as it stands: keep it exactly so, whatever is
    /// learned or changed later.
    pub fn confirm(&self, session: usize) -> Hint {
        Hint::Fixed {
            name: self.tasks[self.sessions[session].task].name.clone(),
            intervals: self.intervals(session),
        }
    }

    /// How this session's task is divided into parts now, as the patterns
    /// a [`Hint::Parts`] gives: each thing in it, with the part it counts
    /// towards (things in no part are left out). More specific patterns
    /// (with a title) come first, so that they decide before a whole app.
    pub fn division(&self, session: usize) -> Vec<(Pattern, String)> {
        let task = self.sessions[session].task;
        let mut out: Vec<(Pattern, String)> = Vec::new();
        for &s in &self.tasks[task].sessions {
            for member in &self.sessions[s].members {
                let pattern = Pattern::of(&member.activity);
                if let Some(part) = &member.part
                    && !out.iter().any(|(p, _)| *p == pattern)
                {
                    out.push((pattern, part.clone()));
                }
            }
        }
        out.sort_by_key(|(p, _)| p.title.is_none());
        out
    }

    /// The hint for a division of this session's task.
    fn parts(&self, session: usize, parts: Vec<(Pattern, String)>) -> Hint {
        Hint::Parts {
            at: self.anchor(session),
            parts,
        }
    }

    /// Divides this session's task into one part per app it uses, named
    /// after the app. `None` if it uses only one.
    pub fn divide_by_app(&self, session: usize) -> Option<Hint> {
        let task = &self.tasks[self.sessions[session].task];
        let mut parts: Vec<(Pattern, String)> = Vec::new();
        for &s in &task.sessions {
            for member in &self.sessions[s].members {
                let pattern = Pattern::app(&member.activity.app);
                if !parts.iter().any(|(p, _)| *p == pattern) {
                    let name = member.name.split(" · ").next().unwrap_or("").to_string();
                    parts.push((pattern, name));
                }
            }
        }
        (parts.len() > 1).then(|| self.parts(session, parts))
    }

    /// Moves one of the things in this session (and wherever else it shows
    /// up in the task) to the part called `part`, a new one or not.
    pub fn move_to_part(&self, session: usize, member: usize, part: &str) -> Hint {
        let activity = &self.sessions[session].members[member].activity;
        let pattern = Pattern::of(activity);
        let mut parts = self.division(session);
        parts.retain(|(p, _)| *p != pattern);
        parts.insert(0, (pattern, part.to_string()));
        parts.sort_by_key(|(p, _)| p.title.is_none());
        self.parts(session, parts)
    }

    /// Gives a part of this session's task another name. Giving it the name
    /// of another part puts the two together.
    pub fn rename_part(&self, session: usize, part: &str, name: &str) -> Hint {
        let mut parts = self.division(session);
        for (_, n) in &mut parts {
            if n == part {
                *n = name.to_string();
            }
        }
        self.parts(session, parts)
    }

    /// Undoes the division of this session's task into parts.
    pub fn undivide(&self, session: usize) -> Hint {
        self.parts(session, Vec::new())
    }

    /// This session's whole task becomes a part of the task of session
    /// `into`, each keeping its time as a part.
    pub fn combine(&self, session: usize, into: usize) -> Option<Hint> {
        (self.sessions[session].task != self.sessions[into].task).then(|| Hint::Combine {
            from: self.anchor(session),
            into: self.anchor(into),
        })
    }

    /// What differs between this timeline and another of the same log.
    pub fn diff(&self, other: &Timeline) -> Diff {
        let key = |timeline: &Timeline, i: usize| {
            let session: &Session = &timeline.sessions[i];
            let name = timeline.tasks[session.task].name.clone();
            let parts = session.parts.clone();
            (timeline.intervals(i), name, session.confirmed, parts)
        };
        let keys = |t: &Timeline| -> Vec<_> { (0..t.sessions.len()).map(|i| key(t, i)).collect() };
        let (mine, theirs) = (keys(self), keys(other));
        Diff {
            gone: (0..mine.len())
                .filter(|&i| !theirs.contains(&mine[i]))
                .collect(),
            new: (0..theirs.len())
                .filter(|&i| !mine.contains(&theirs[i]))
                .collect(),
        }
    }
}
