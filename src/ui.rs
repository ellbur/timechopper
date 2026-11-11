//! The live window: a small panel, meant for a corner of the screen, that
//! shows the day as it is being interpreted, right up to the window in focus
//! now, and lets the user put the interpretation right as they go.
//!
//! [`Live`] holds everything; it takes focus changes from a channel (fed by
//! the watcher, or by a replay of a synthetic log), keeps them in a
//! [`Journal`], and re-interprets whenever something changes. Each frame
//! shows:
//!
//! - **Now**: the window in focus and the row it is part of, and a box to
//!   mark the moment (a note, or a break). A marker is not a task: it is
//!   evidence of where one may have ended, which the guess weighs and the
//!   user can come back to.
//! - **Rows**: newest first, carve-outs indented under the row they
//!   interrupted. A row that just changed shape flashes. Clicking a row opens
//!   what it is made of and what can be done to it; hovering over an action
//!   outlines the rows it would change.
//! - **Log**: focus changes and markers (the user's, and windows opened and
//!   closed), newest first, each focus change in the colour of its task.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Color32, CornerRadius, Frame, Layout, Margin, RichText, ScrollArea, Sense, Stroke,
    TextEdit, Ui,
};

use crate::interpret::history::{self, Precedent};
use crate::interpret::model::{Event, Time, fmt_duration};
use crate::interpret::{
    Diff, Doubt, Hint, Journal, Mark, Marker, Member, Record, Scope, Timeline, Why,
};
use crate::live::{Clock, LogFile, Observation};

/// The colour of what may need the user's attention.
const DOUBT: Color32 = Color32::from_rgb(0xf2, 0x8e, 0x2b);

/// The colour of the user's markers.
const MARKER: Color32 = Color32::from_rgb(0x76, 0xb7, 0xb2);

/// How long a row that changed shape stays highlighted.
const FLASH: Duration = Duration::from_millis(2500);
/// How many focus changes the log at the bottom shows.
const RECENT: usize = 200;

/// A line of the log at the bottom: a focus change (a span, and when it
/// ended) or a marker.
enum Line {
    Focus(usize, Time),
    Marker(usize),
}

/// A row, identified across re-interpretations by when it first had focus.
type Key = Time;

/// Something the user asked for while a frame was being drawn.
enum Act {
    Apply(Hint),
    Mark(Mark),
    Undo,
    Toggle(Key),
}

/// Something the user did that Undo can take back.
enum Done {
    Hint,
    /// A marker, made at this time.
    Mark(Time),
}

pub struct Live {
    journal: Journal,
    feed: Receiver<Observation>,
    clock: Arc<dyn Clock>,
    log: Option<LogFile>,

    /// The interpretation as of `now`.
    timeline: Arc<Timeline>,
    now: Time,
    /// Whether anything other than the passing of time changed since
    /// `timeline` was made.
    stale: bool,
    /// The last focus change, which tells what is in focus now.
    current: Option<Event>,
    /// What Undo would take back, latest last.
    done: Vec<Done>,

    /// Rows that changed shape lately, and when.
    flashes: HashMap<Key, Instant>,
    /// The shape of each row last time, to see which changed.
    shapes: HashMap<Key, Shape>,
    open: HashSet<Key>,

    /// The action under the mouse, and the rows of the current timeline it
    /// would change.
    preview: Option<(Hint, Diff)>,
    /// The action under the mouse in the frame being drawn, and in the one
    /// before (rows are drawn before the actions in them).
    hovered: Option<Hint>,
    was_hovered: Option<Hint>,

    note: String,
    naming: Option<(Key, String)>,
    split_text: HashMap<Key, String>,
    status: Option<String>,
    placed: bool,
}

/// What makes a row look different to the user: when it started, what went
/// into it, what it is called, where it sits.
#[derive(Clone, PartialEq, Eq)]
struct Shape {
    pieces: usize,
    task: Option<String>,
    parent: Option<Key>,
    confirmed: bool,
    headline: String,
}

impl Live {
    /// A live view of the focus changes arriving on `feed`, starting from
    /// those that happened earlier (from a log file being continued).
    pub fn new(
        journal: Journal,
        feed: Receiver<Observation>,
        clock: Arc<dyn Clock>,
        log: Option<LogFile>,
    ) -> Live {
        let now = clock.now();
        let mut live = Live {
            journal,
            feed,
            clock,
            log,
            timeline: Arc::default(),
            now,
            stale: true,
            current: None,
            done: Vec::new(),
            flashes: HashMap::new(),
            shapes: HashMap::new(),
            open: HashSet::new(),
            preview: None,
            hovered: None,
            was_hovered: None,
            note: String::new(),
            naming: None,
            split_text: HashMap::new(),
            status: None,
            placed: false,
        };
        live.refresh();
        // Nothing has "changed" on the first look.
        live.flashes.clear();
        live
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    /// Takes in whatever has arrived on the feed and re-interprets if
    /// anything (including the time) has moved on.
    pub fn refresh(&mut self) {
        while let Ok(observation) = self.feed.try_recv() {
            self.observe(observation);
        }
        let now = self.clock.now().max(self.now);
        if now == self.now && !self.stale {
            return;
        }
        self.now = now;
        self.stale = false;
        self.timeline = Arc::new(self.journal.timeline(now));
        self.preview = None;
        self.note_changes();
    }

    fn observe(&mut self, record: Observation) {
        let record = match record {
            // The journal needs focus changes in order; one can come out of
            // order only if the clock was put back.
            Record::Focus(event) => {
                let at = event.at.max(self.journal.latest().unwrap_or(Time(0)));
                let event = Event { at, ..event };
                self.current = Some(event.clone());
                Record::Focus(event)
            }
            other => other,
        };
        if let Some(log) = &mut self.log
            && let Err(e) = log.write(&record)
        {
            self.status = Some(format!("could not write {}: {e}", log.path().display()));
        }
        self.journal.record(record);
        self.stale = true;
    }

    /// The user marks this moment: with a note, or as the start of a break.
    /// It is kept in the log with everything else.
    pub fn mark(&mut self, mark: Mark) {
        let at = self.clock.now().max(self.now);
        self.observe(Record::Mark(Marker { at, mark }));
        self.done.push(Done::Mark(at));
        self.refresh();
    }

    /// Applies a correction now. Like everything else, it is kept in the
    /// log, so it still holds after a restart.
    pub fn apply(&mut self, hint: Hint) {
        let at = self.clock.now().max(self.now);
        self.observe(Record::Hint { at, hint });
        self.done.push(Done::Hint);
        self.refresh();
    }

    /// Takes in what an earlier run of the window logged (from a log file
    /// being continued), as if it had happened here: corrections and markers
    /// made then can be undone now.
    pub fn resume(&mut self, records: Vec<Record>) {
        for record in records {
            match &record {
                Record::Mark(m) if m.mark.by_user() => self.done.push(Done::Mark(m.at)),
                Record::Unmark { mark, .. } => {
                    let taken = self
                        .done
                        .iter()
                        .rposition(|d| matches!(d, Done::Mark(t) if t == mark));
                    if let Some(i) = taken {
                        self.done.remove(i);
                    }
                }
                Record::Hint { .. } => self.done.push(Done::Hint),
                Record::Unhint { .. } => {
                    if let Some(i) = self.done.iter().rposition(|d| matches!(d, Done::Hint)) {
                        self.done.remove(i);
                    }
                }
                Record::Focus(event) => self.current = Some(event.clone()),
                Record::Mark(_) => {}
            }
            self.journal.record(record);
        }
        self.stale = true;
        self.refresh();
        self.flashes.clear();
    }

    /// Takes back the latest correction or marker.
    pub fn undo(&mut self) {
        match self.done.pop() {
            Some(Done::Hint) => {
                let at = self.clock.now().max(self.now);
                self.observe(Record::Unhint { at });
            }
            Some(Done::Mark(mark)) => {
                let at = self.clock.now().max(self.now);
                self.observe(Record::Unmark { at, mark });
            }
            None => return,
        }
        self.stale = true;
        self.refresh();
    }

    /// What Undo would take back, in words.
    fn undoable(&self) -> Option<String> {
        match self.done.last()? {
            Done::Hint => self.journal.hints().last().map(describe_hint),
            Done::Mark(at) => {
                let marker = self.journal.marks().iter().rfind(|m| m.at == *at)?;
                Some(format!("Take back: {} at {}", marker.mark, at.hm()))
            }
        }
    }

    fn shape(&self, i: usize) -> Shape {
        let t = &self.timeline;
        let session = &t.sessions[i];
        Shape {
            pieces: session.spans.len(),
            task: t.tasks[session.task].name.clone(),
            parent: session.parent.map(|p| t.anchor(p)),
            confirmed: session.confirmed,
            headline: session.label.headline.clone(),
        }
    }

    /// Notes which rows came out differently this time. A row merely
    /// running on (the window in focus staying in focus) has not changed.
    fn note_changes(&mut self) {
        let mut shapes = HashMap::new();
        for i in 0..self.timeline.sessions.len() {
            let key = self.timeline.anchor(i);
            let shape = self.shape(i);
            if self.shapes.get(&key) != Some(&shape) {
                self.flashes.insert(key, Instant::now());
            }
            shapes.insert(key, shape);
        }
        self.shapes = shapes;
        self.flashes.retain(|_, at| at.elapsed() < FLASH);
    }

    /// The session holding the window in focus now, if one is.
    pub fn current_session(&self) -> Option<usize> {
        let window = self.current.as_ref()?.window.as_ref()?;
        let t = &self.timeline;
        let last = t.spans.len().checked_sub(1)?;
        (t.spans[last].end == self.now && t.spans[last].window == *window)
            .then(|| t.sessions.iter().position(|s| s.spans.contains(&last)))?
    }

    // ---- drawing ----

    fn color(&self, task: usize) -> Color32 {
        const PALETTE: [Color32; 10] = [
            Color32::from_rgb(0x4e, 0x79, 0xa7),
            Color32::from_rgb(0xf2, 0x8e, 0x2b),
            Color32::from_rgb(0x59, 0xa1, 0x4f),
            Color32::from_rgb(0xe1, 0x57, 0x59),
            Color32::from_rgb(0x76, 0xb7, 0xb2),
            Color32::from_rgb(0xed, 0xc9, 0x48),
            Color32::from_rgb(0xb0, 0x7a, 0xa1),
            Color32::from_rgb(0xff, 0x9d, 0xa7),
            Color32::from_rgb(0x9c, 0x75, 0x5f),
            Color32::from_rgb(0xba, 0xb0, 0xac),
        ];
        // Tasks are numbered by first appearance, so a task keeps its
        // colour as the day goes on, unless an earlier one is regrouped.
        PALETTE[task % PALETTE.len()]
    }

    /// A button for a correction. Hovering over it outlines the rows it
    /// would change; clicking it queues it.
    fn action(&mut self, ui: &mut Ui, label: &str, hint: Option<Hint>, acts: &mut Vec<Act>) {
        let enabled = hint.is_some();
        let response = ui.add_enabled(enabled, egui::Button::new(label).small());
        let Some(hint) = hint else { return };
        let response = response.on_hover_text(describe_hint(&hint));
        if response.hovered() {
            self.hovered = Some(hint.clone());
        }
        if response.clicked() {
            acts.push(Act::Apply(hint));
        }
    }

    /// Rows (sessions of [`Live::timeline`]) that the action under the mouse
    /// would change, outlined while it is there.
    pub fn previewed(&self) -> &[usize] {
        match &self.preview {
            Some((hint, diff)) if self.was_hovered.as_ref() == Some(hint) => &diff.gone,
            _ => &[],
        }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        self.refresh();
        self.was_hovered = self.hovered.take();
        let mut acts = Vec::new();

        egui::Panel::top("now").show(ui, |ui| self.now_panel(ui, &mut acts));
        egui::Panel::bottom("log")
            .resizable(true)
            .default_size(170.0)
            .show(ui, |ui| self.log_panel(ui, &mut acts));
        egui::CentralPanel::default().show(ui, |ui| self.rows_panel(ui, &mut acts));

        // Work out what the action under the mouse would do, for the next
        // frame to show.
        match &self.hovered {
            Some(hint) if self.preview.as_ref().is_none_or(|(h, _)| h != hint) => {
                let (_, diff) = self.journal.preview(hint.clone(), self.now);
                self.preview = Some((hint.clone(), diff));
            }
            _ => {}
        }
        if self.hovered != self.was_hovered {
            ui.ctx().request_repaint();
        }

        for act in acts {
            match act {
                Act::Apply(hint) => self.apply(hint),
                Act::Mark(mark) => self.mark(mark),
                Act::Undo => self.undo(),
                Act::Toggle(key) => {
                    if !self.open.remove(&key) {
                        self.open.insert(key);
                    }
                }
            }
        }

        // Keep the clock in the corner ticking, and fade the highlights.
        let tick = match self.flashes.is_empty() {
            true => Duration::from_secs(1),
            false => Duration::from_millis(100),
        };
        ui.ctx().request_repaint_after(tick);
    }

    fn now_panel(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(self.now.to_string()).monospace().strong());
            match self
                .current
                .as_ref()
                .and_then(|c| c.window.as_ref().map(|w| (c.at, w)))
            {
                Some((since, window)) => {
                    let app = crate::interpret::title::app_stem(&window.app);
                    ui.label(RichText::new(app).weak());
                    ui.label(&window.title)
                        .on_hover_text(format!("{}\nin focus since {since}", window.app));
                }
                None => {
                    ui.label(RichText::new("nothing in focus").weak().italics());
                }
            }
        });
        let current = self.current_session();
        let timeline = self.timeline.clone();
        ui.horizontal(|ui| match current {
            Some(i) => {
                let session = &timeline.sessions[i];
                let task = &timeline.tasks[session.task];
                swatch(ui, self.color(session.task));
                ui.label(RichText::new(title_of(&timeline, i)).strong());
                let total = match task.active > session.active {
                    true => format!(" ({} today)", fmt_duration(task.active)),
                    false => String::new(),
                };
                ui.label(format!("{}{total}", fmt_duration(session.active)));
            }
            None if self.current.as_ref().is_some_and(|c| c.window.is_some()) => {
                ui.label(RichText::new("not part of any row yet").weak());
            }
            None => {}
        });
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let last = self.undoable();
                let undo = ui.add_enabled(last.is_some(), egui::Button::new("Undo"));
                if undo.on_hover_text(last.unwrap_or_default()).clicked() {
                    acts.push(Act::Undo);
                }
                let rest = ui
                    .button("Break")
                    .on_hover_text("Mark the start of a break (with the note, if any)")
                    .clicked();
                let mark = ui
                    .button("Mark")
                    .on_hover_text(
                        "Mark this moment, with the note if any: something changed here. \
                         It helps find where tasks begin and end; it doesn't start one",
                    )
                    .clicked();
                let edit = TextEdit::singleline(&mut self.note)
                    .hint_text("note (optional)")
                    .desired_width(ui.available_width());
                let response = ui.add(edit);
                let entered =
                    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if mark || entered || rest {
                    let text = std::mem::take(&mut self.note).trim().to_string();
                    acts.push(Act::Mark(match rest {
                        true => Mark::Break(text),
                        false => Mark::Note(text),
                    }));
                }
            });
        });
        if let Some(status) = &self.status {
            ui.colored_label(Color32::from_rgb(0xe1, 0x57, 0x59), status);
        }
        ui.add_space(2.0);
    }

    fn rows_panel(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        let timeline = self.timeline.clone();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Rows").strong());
            let doubts = (0..timeline.sessions.len())
                .filter(|&i| doubt_count(&timeline, i) > 0)
                .count();
            let mut notes = vec![format!("{} tracked", fmt_duration(timeline.tracked))];
            if doubts > 0 {
                notes.push(format!("{doubts} to check"));
            }
            ui.label(RichText::new(notes.join(" · ")).weak());
        });
        // Newest first, carve-outs under the row they interrupted.
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); timeline.sessions.len()];
        let mut roots = Vec::new();
        for (i, session) in timeline.sessions.iter().enumerate() {
            match session.parent {
                Some(p) => children[p].push(i),
                None => roots.push(i),
            }
        }
        let mut order = Vec::new();
        let mut stack: Vec<(usize, usize)> = roots.into_iter().map(|i| (i, 0)).collect();
        while let Some((i, depth)) = stack.pop() {
            order.push((i, depth));
            stack.extend(children[i].iter().map(|&c| (c, depth + 1)));
        }
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if order.is_empty() {
                    ui.label(RichText::new("Nothing yet: rows appear as you work.").weak());
                }
                for (i, depth) in order {
                    self.row(ui, &timeline, i, depth, acts);
                }
            });
    }

    fn row(&mut self, ui: &mut Ui, t: &Timeline, i: usize, depth: usize, acts: &mut Vec<Act>) {
        let session = &t.sessions[i];
        let task = &t.tasks[session.task];
        let key = t.anchor(i);
        let open = self.open.contains(&key);
        let live = self.current_session() == Some(i);

        let flash = self.flashes.get(&key).map_or(0.0, |at| {
            1.0 - at.elapsed().as_secs_f32() / FLASH.as_secs_f32()
        });
        let visuals = ui.visuals().clone();
        let fill = match flash > 0.0 {
            true => visuals
                .faint_bg_color
                .lerp_to_gamma(Color32::from_rgb(0xed, 0xc9, 0x48), 0.3 * flash),
            false => visuals.faint_bg_color,
        };
        let stroke = match self.previewed().contains(&i) {
            true => Stroke::new(2.0, Color32::from_rgb(0xe1, 0x57, 0x59)),
            false if live => Stroke::new(1.0, visuals.widgets.active.bg_stroke.color),
            false => Stroke::NONE,
        };

        {
            Frame::new()
                .fill(fill)
                .stroke(stroke)
                .corner_radius(CornerRadius::same(4))
                .inner_margin(Margin::symmetric(6, 3))
                .outer_margin(Margin {
                    left: (16 * depth.min(4)) as i8,
                    bottom: 2,
                    ..Margin::ZERO
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let header = ui.horizontal(|ui| {
                        swatch(ui, self.color(session.task));
                        let end = match live {
                            true => "now".to_string(),
                            false => session.end.hm(),
                        };
                        ui.label(
                            RichText::new(format!("{}–{end}", session.start.hm())).monospace(),
                        );
                        ui.label(RichText::new(fmt_duration(session.active)).monospace());
                        let title = title_of(t, i);
                        let name = match task.name.is_some() {
                            true => RichText::new(title).strong(),
                            false => RichText::new(title),
                        };
                        let doubts = doubt_count(t, i);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if doubts > 0 {
                                ui.label(
                                    RichText::new(format!("? {doubts}"))
                                        .color(Color32::from_rgb(0xf2, 0x8e, 0x2b))
                                        .strong(),
                                )
                                .on_hover_text("Something in this row may not belong");
                            }
                            if session.confirmed {
                                ui.label(RichText::new("confirmed").small().weak())
                                    .on_hover_text("You confirmed this row: it stays as it is");
                            }
                            // The title takes whatever room is left.
                            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                ui.add(egui::Label::new(name).truncate().sense(Sense::click()))
                            })
                            .inner
                        })
                        .inner
                    });
                    let mut notes = vec![session.label.apps.join(", ")];
                    if task.sessions.len() > 1 {
                        let nth = task.sessions.iter().position(|&s| s == i).unwrap_or(0) + 1;
                        notes.push(format!("part {nth} of {}", task.sessions.len()));
                    }
                    if session.elsewhere() > 0 {
                        notes.push(format!("{} elsewhere", fmt_duration(session.elsewhere())));
                    }
                    if let [member] = &session.members[..]
                        && let Some(p) = member.precedent.as_ref().filter(|p| p.applied)
                    {
                        notes.push(format!("own task, as on {}", history::days(&p.days)));
                    }
                    let summary = ui.add(
                        egui::Label::new(RichText::new(notes.join(" · ")).small().weak())
                            .truncate()
                            .sense(Sense::click()),
                    );
                    if header.inner.clicked() || summary.clicked() {
                        acts.push(Act::Toggle(key));
                    }
                    self.markers(ui, t, i, open, acts);
                    if open {
                        self.details(ui, t, i, acts);
                    }
                });
        }
    }

    /// The user's markers in a row, always shown: they are the user's own
    /// account of the day. With the row open, each can be acted on.
    fn markers(&mut self, ui: &mut Ui, t: &Timeline, i: usize, open: bool, acts: &mut Vec<Act>) {
        let session = &t.sessions[i];
        // What earlier days say this row's task is called.
        let task = &t.tasks[session.task];
        if let Some((name, p)) = offered_name(t, i) {
            ui.horizontal(|ui| {
                let text = format!("? called \"{name}\" on {}", history::days(&p.days));
                ui.label(RichText::new(text).small().color(DOUBT));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    self.action(ui, "use", Some(t.name(i, &name)), acts);
                });
            });
        }
        if let Some(p) = task
            .precedent
            .as_ref()
            .filter(|p| p.applied && p.name == task.name)
            && open
        {
            ui.horizontal(|ui| {
                let text = format!("named as on {}", history::days(&p.days));
                ui.label(RichText::new(text).small().weak());
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    self.action(ui, "not this", Some(t.name(i, "")), acts);
                });
            });
        }
        let starts = session.opened_by.into_iter().map(|k| (k, true));
        for (k, first) in starts.chain(session.marks.iter().map(|&k| (k, false))) {
            let marker = &t.marks[k];
            ui.horizontal(|ui| {
                let text = match first {
                    true => format!("> {}  {}  (starts here)", marker.at.hm(), marker.mark),
                    false => format!("> {}  {}", marker.at.hm(), marker.mark),
                };
                ui.label(RichText::new(text).small().color(MARKER));
                if !open {
                    return;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // What a row starts with may say what it is; something
                    // marked partway through may be where another begins.
                    match (first, marker.mark.text()) {
                        (true, Some(text))
                            if t.tasks[session.task].name.as_deref() != Some(text) =>
                        {
                            self.action(ui, "use as name", Some(t.name(i, text)), acts)
                        }
                        (true, _) => {}
                        (false, _) => self.action(ui, "split here", t.split(i, marker.at), acts),
                    }
                });
            });
        }
    }

    fn details(&mut self, ui: &mut Ui, t: &Timeline, i: usize, acts: &mut Vec<Act>) {
        let session = &t.sessions[i];
        let key = t.anchor(i);
        ui.separator();
        for (m, member) in session.members.iter().enumerate() {
            ui.horizontal(|ui| {
                let doubtful = doubtful(member);
                let marker = match doubtful {
                    true => RichText::new("?")
                        .color(Color32::from_rgb(0xf2, 0x8e, 0x2b))
                        .strong(),
                    false => RichText::new("·").weak(),
                };
                ui.label(marker);
                ui.label(&member.name).on_hover_text(why_text(member.why));
                let visits = match member.visits {
                    1 => "1 visit".to_string(),
                    n => format!("{n} visits"),
                };
                ui.label(
                    RichText::new(format!("{} · {visits}", fmt_duration(member.secs)))
                        .small()
                        .weak(),
                );
                if session.members.len() > 1 && !session.confirmed {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.menu_button("own task…", |ui| {
                            let here = t.separate(i, m, Scope::Here);
                            self.action(ui, "just here", Some(here), acts);
                            let everywhere = t.separate(i, m, Scope::Everywhere);
                            self.action(ui, "wherever it shows up", Some(everywhere), acts);
                        })
                        .response
                        .on_hover_text("Take this out of the row: it is a task of its own");
                    });
                }
            });
            if let Some(p) = member.precedent.as_ref().filter(|p| !p.applied) {
                let text = format!("    taken out of its row on {}", history::days(&p.days));
                ui.label(RichText::new(text).small().weak());
            }
            for doubt in &member.doubts {
                let text = match doubt {
                    Doubt::Elsewhere { own: true, .. } => "also a task of its own".to_string(),
                    Doubt::Elsewhere { tasks: 1, .. } => "also in another task".to_string(),
                    Doubt::Elsewhere { tasks, .. } => format!("also in {tasks} other tasks"),
                };
                let text = match member.why {
                    Why::SameApp => format!("seen once, in an app the task uses; {text}"),
                    _ => text,
                };
                ui.label(RichText::new(format!("    {text}")).small().weak());
            }
        }
        for split in &session.splits {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("?")
                        .color(Color32::from_rgb(0xf2, 0x8e, 0x2b))
                        .strong(),
                );
                ui.label(format!(
                    "{}  {} may give way to {}",
                    split.at.hm(),
                    split.before,
                    split.after
                ));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    self.action(ui, "split here", t.split(i, split.at), acts);
                });
            });
        }
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            // Name.
            match &mut self.naming {
                Some((k, text)) if *k == key => {
                    let edit = ui.add(
                        TextEdit::singleline(text)
                            .hint_text("what to call it")
                            .desired_width(120.0),
                    );
                    edit.request_focus();
                    let entered =
                        edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let name = text.trim().to_string();
                    let hint = (!name.is_empty()).then(|| t.name(i, &name));
                    if entered && let Some(hint) = hint.clone() {
                        acts.push(Act::Apply(hint));
                        self.naming = None;
                    } else {
                        let before = acts.len();
                        self.action(ui, "OK", hint, acts);
                        if acts.len() > before {
                            self.naming = None;
                        }
                    }
                    if ui.small_button("Cancel").clicked() {
                        self.naming = None;
                    }
                }
                _ => {
                    if ui.small_button("Name…").clicked() {
                        let name = t.tasks[session.task].name.clone().unwrap_or_default();
                        self.naming = Some((key, name));
                    }
                }
            }
            if !session.confirmed {
                self.action(ui, "Confirm", Some(t.confirm(i)), acts);
            }
            // Join into another row's task.
            ui.menu_button("Join into…", |ui| {
                let mut others: Vec<usize> = (0..t.sessions.len())
                    .filter(|&j| t.sessions[j].task != session.task)
                    .collect();
                others.sort_by_key(|&j| t.sessions[j].start.0.abs_diff(session.start.0));
                if others.is_empty() {
                    ui.label(RichText::new("no other rows").weak());
                }
                for j in others.into_iter().take(12) {
                    let label = format!("{}  {}", t.sessions[j].start.hm(), title_of(t, j));
                    self.action(ui, &label, t.join(i, j), acts);
                }
            });
            // Split at a time typed in.
            let text = self.split_text.entry(key).or_default();
            ui.add(
                TextEdit::singleline(text)
                    .hint_text("HH:MM")
                    .desired_width(48.0),
            );
            let at = Time::parse(text.trim());
            let hint = at.and_then(|at| t.split(i, at));
            let before = acts.len();
            self.action(ui, "Split", hint, acts);
            if acts.len() > before {
                self.split_text.remove(&key);
            }
        });
    }

    fn log_panel(&mut self, ui: &mut Ui, acts: &mut Vec<Act>) {
        let t = self.timeline.clone();
        ui.add_space(2.0);
        ui.label(RichText::new("Log").strong());
        let mut owner = vec![None; t.spans.len()];
        for (i, session) in t.sessions.iter().enumerate() {
            for &s in &session.spans {
                owner[s] = Some(i);
            }
        }
        // Spans cut in two by a correction read as one focus change, unless
        // the cut moved the second part to another row.
        let mut entries: Vec<(usize, Time)> = Vec::new();
        for (s, span) in t.spans.iter().enumerate() {
            match entries.last_mut() {
                Some((last, end))
                    if *end == span.start
                        && t.spans[*last].window == span.window
                        && owner[*last] == owner[s] =>
                {
                    *end = span.end
                }
                _ => entries.push((s, span.end)),
            }
        }
        let live = self.current_session().is_some();
        let count = entries.len() + t.marks.len();
        // Markers between the focus changes, at their times.
        let mut lines: Vec<Line> = entries
            .iter()
            .map(|&(s, end)| Line::Focus(s, end))
            .collect();
        lines.extend((0..t.marks.len()).map(Line::Marker));
        let at = |line: &Line| match *line {
            Line::Focus(s, _) => (t.spans[s].start, 1),
            Line::Marker(k) => (t.marks[k].at, 0),
        };
        lines.sort_by_key(at);
        let newest_focus = entries.len().checked_sub(1).map(|n| entries[n].0);
        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for line in lines.iter().rev().take(RECENT) {
                    let (s, end) = match *line {
                        Line::Focus(s, end) => (s, end),
                        Line::Marker(k) => {
                            let marker = &t.marks[k];
                            ui.horizontal(|ui| {
                                swatch(ui, Color32::TRANSPARENT);
                                ui.label(RichText::new(marker.at.to_string()).monospace().small());
                                let text = RichText::new(marker.mark.to_string()).small();
                                let text = match marker.mark.by_user() {
                                    true => text.color(MARKER).strong(),
                                    false => text.weak().italics(),
                                };
                                ui.add(egui::Label::new(text).truncate());
                            });
                            continue;
                        }
                    };
                    let span = &t.spans[s];
                    let n = usize::from(Some(s) != newest_focus);
                    ui.horizontal(|ui| {
                        match owner[s] {
                            Some(i) => swatch(ui, self.color(t.sessions[i].task)),
                            None => swatch(ui, Color32::TRANSPARENT),
                        }
                        ui.label(RichText::new(span.start.to_string()).monospace().small());
                        let secs = end.0 - span.start.0;
                        let length = match n == 0 && live {
                            true => format!("{} so far", fmt_duration(secs)),
                            false => fmt_duration(secs),
                        };
                        ui.label(
                            RichText::new(format!("{length:>6}"))
                                .monospace()
                                .small()
                                .weak(),
                        );
                        let app = crate::interpret::title::app_stem(&span.window.app);
                        ui.label(RichText::new(app).small().weak());
                        let title = ui.add(
                            egui::Label::new(RichText::new(&span.window.title).small())
                                .truncate()
                                .sense(Sense::click()),
                        );
                        if let Some(i) = owner[s]
                            && title
                                .on_hover_text(format!("in: {}", title_of(&t, i)))
                                .clicked()
                        {
                            let key = t.anchor(i);
                            if !self.open.contains(&key) {
                                acts.push(Act::Toggle(key));
                            }
                        }
                    });
                }
                if count == 0 {
                    ui.label(RichText::new("Waiting for the first window…").weak());
                }
            });
    }

    /// Puts the window in the bottom right corner of the screen, once the
    /// screen's size is known.
    fn place(&mut self, ctx: &egui::Context) {
        if self.placed {
            return;
        }
        let (monitor, outer) = ctx.input(|i| (i.viewport().monitor_size, i.viewport().outer_rect));
        if let (Some(monitor), Some(outer)) = (monitor, outer) {
            let margin = egui::vec2(12.0, 56.0); // clear of the taskbar
            let pos = monitor - outer.size() - margin;
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos.to_pos2()));
            self.placed = true;
        }
    }
}

impl eframe::App for Live {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.place(ui.ctx());
        self.show(ui);
    }
}

fn swatch(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 14.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(2), color);
}

/// What a row is called: the user's name for its task, else its headline.
fn title_of(t: &Timeline, i: usize) -> String {
    let session = &t.sessions[i];
    match &t.tasks[session.task].name {
        Some(name) => name.clone(),
        None => session.label.headline.clone(),
    }
}

fn doubt_count(t: &Timeline, i: usize) -> usize {
    let session = &t.sessions[i];
    let members = session.members.iter().filter(|m| doubtful(m));
    members.count() + session.splits.len() + usize::from(offered_name(t, i).is_some())
}

/// Whether something swept into a row may not belong there: it has an
/// identity of its own, or it was taken out of rows on earlier days.
fn doubtful(member: &Member) -> bool {
    !member.doubts.is_empty() || member.precedent.as_ref().is_some_and(|p| !p.applied)
}

/// The name earlier days offer for a row's task, if it has none yet.
fn offered_name(t: &Timeline, i: usize) -> Option<(String, &Precedent)> {
    let task = &t.tasks[t.sessions[i].task];
    let p = task.precedent.as_ref()?;
    (task.name.is_none() && !p.applied).then(|| (p.name.clone().unwrap_or_default(), p))
}

fn why_text(why: Why) -> &'static str {
    match why {
        Why::Main => "More time went into it than into anything else here",
        Why::Returned => "You kept coming back to it",
        Why::SameApp => "Seen once, in an app this task uses",
        Why::Once => "Seen once here; it belongs to this task elsewhere",
        Why::Confirmed => "You confirmed this row",
    }
}

/// A correction, in words.
pub fn describe_hint(hint: &Hint) -> String {
    let ranges = |during: &[(Time, Time)]| {
        let parts: Vec<String> = during
            .iter()
            .map(|(a, b)| format!("{}–{}", a.hm(), b.hm()))
            .collect();
        parts.join(", ")
    };
    match hint {
        Hint::Lens { app, lens } => format!("Tell {app} windows apart by {lens:?}"),
        Hint::Apart { pattern, during } if during.is_empty() => {
            format!("{pattern} is always a task of its own")
        }
        Hint::Apart { pattern, during } => {
            format!("{pattern} is a task of its own at {}", ranges(during))
        }
        Hint::Together(a, b) => format!("{a} and {b} are the same thing"),
        Hint::Fixed { intervals, .. } => format!("Keep the row at {} as it is", ranges(intervals)),
        Hint::Split { at } => format!("A new task from {at}"),
        Hint::Join { from, into } => {
            format!(
                "The row at {} is part of the task at {}",
                from.hm(),
                into.hm()
            )
        }
        Hint::Name { at, name } if name.is_empty() => {
            format!("Take away the name of the task at {}", at.hm())
        }
        Hint::Name { at, name } => format!("Call the task at {} \"{name}\"", at.hm()),
        Hint::Usual { activity, name } => format!("Work on {activity} is called \"{name}\""),
    }
}
