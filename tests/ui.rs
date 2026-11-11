//! The live window, driven headlessly: focus changes arrive on the feed,
//! the clock is moved by hand, and buttons are found and clicked through
//! the accessibility tree.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};

use eframe::egui;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use timechopper::interpret::model::fmt_duration;
use timechopper::interpret::scenarios::{self, FIREFOX, NOTEPAD, OUTLOOK};
use timechopper::interpret::{
    Event, Journal, Mark, Marker, Params, Record, Scope, Span, Time, Window,
};
use timechopper::live::{self, FnClock, LogFile, Observation};
use timechopper::ui::Live;

struct Rig {
    send: Sender<Observation>,
    clock: Arc<AtomicU64>,
}

impl Rig {
    fn at(&self, t: Time) {
        self.clock.store(t.0, Ordering::SeqCst);
    }

    fn focus(&self, t: Time, app: &str, title: &str) {
        let window = Window {
            app: app.into(),
            title: title.into(),
        };
        self.send
            .send(Record::Focus(Event {
                at: t,
                window: Some(window),
            }))
            .unwrap();
        self.at(t);
    }

    /// Feeds a whole log, leaving the clock at its end with its last
    /// window still in focus.
    fn play(&self, spans: &[Span]) {
        self.play_marked(spans, &[]);
    }

    fn play_marked(&self, spans: &[Span], marks: &[Marker]) {
        let mut records = live::records(spans, marks);
        let end = live::at(&records.pop().unwrap());
        for record in records {
            self.at(live::at(&record));
            self.send.send(record).unwrap();
        }
        self.at(end);
    }
}

fn harness() -> (Harness<'static, Live>, Rig) {
    harness_logged(None, Time::hms(9, 0, 0))
}

/// A window keeping its log in the file at `log` (carrying on from what is
/// already in it), with the clock at `now`.
fn harness_logged(log: Option<&std::path::Path>, now: Time) -> (Harness<'static, Live>, Rig) {
    harness_with(Journal::new(Params::default()), log, now)
}

fn harness_with(
    journal: Journal,
    log: Option<&std::path::Path>,
    now: Time,
) -> (Harness<'static, Live>, Rig) {
    let (send, feed) = mpsc::channel();
    let clock = Arc::new(AtomicU64::new(now.0));
    let reading = clock.clone();
    let clock_fn = FnClock(move || Time(reading.load(Ordering::SeqCst)));
    let (log, earlier) = match log {
        Some(path) => {
            let (log, earlier) = LogFile::open(path).unwrap();
            (Some(log), earlier)
        }
        None => (None, Vec::new()),
    };
    let mut live = Live::new(journal, feed, Arc::new(clock_fn), log);
    live.resume(earlier);
    let harness = Harness::builder()
        .with_size(egui::vec2(400.0, 620.0))
        .build_ui_state(|ui, live: &mut Live| live.show(ui), live);
    (harness, Rig { send, clock })
}

fn t(text: &str) -> Time {
    Time::parse(text).unwrap()
}

#[test]
fn focus_changes_show_up_as_they_happen() {
    let (mut h, rig) = harness();
    h.run();
    assert!(h.query_by_label("Waiting for the first window…").is_some());

    rig.focus(t("09:00"), NOTEPAD, "report.txt - Notepad");
    rig.at(t("09:03"));
    h.run();
    // The log, and the "now" line, show the window in focus.
    assert!(h.query_all_by_label("report.txt - Notepad").count() >= 2);
    assert!(h.query_by_label("3m so far").is_some());
    assert_eq!(h.state().timeline().sessions.len(), 1);
    assert_eq!(h.state().current_session(), Some(0));

    // Back and forth with the browser: one row, growing.
    rig.focus(
        t("09:03"),
        FIREFOX,
        "Quarterly report examples — Mozilla Firefox",
    );
    rig.focus(t("09:05"), NOTEPAD, "report.txt - Notepad");
    rig.focus(
        t("09:09"),
        FIREFOX,
        "Quarterly report examples — Mozilla Firefox",
    );
    rig.focus(t("09:10"), NOTEPAD, "report.txt - Notepad");
    rig.at(t("09:14"));
    h.run();
    let timeline = h.state().timeline();
    assert_eq!(timeline.sessions.len(), 1, "{timeline:#?}");
    assert_eq!(timeline.sessions[0].active, 14 * 60);
    assert!(h.query_by_label("09:00–now").is_some());
    assert!(h.query_by_label("4m so far").is_some());

    // A glance at mail is carved out of it.
    rig.focus(t("09:14"), OUTLOOK, "Inbox - Outlook");
    rig.focus(t("09:14:30"), NOTEPAD, "report.txt - Notepad");
    rig.at(t("09:20"));
    h.run();
    let timeline = h.state().timeline();
    assert_eq!(timeline.sessions.len(), 2);
    let mail = timeline
        .sessions
        .iter()
        .find(|s| s.parent.is_some())
        .unwrap();
    assert_eq!(mail.active, 30);
    assert!(h.query_by_label("Inbox - Outlook").is_some());
}

#[test]
fn marking_a_moment_records_it_without_starting_a_task() {
    let (mut h, rig) = harness();
    rig.play(&(scenarios::ALL[0].spans)()); // alternation, still going
    h.run();
    let before = h.state().timeline().clone();
    let now = before.sessions[0].end;

    h.get_by_role(Role::TextInput).click();
    h.run();
    h.get_by_role(Role::TextInput).type_text("Budget");
    h.run();
    h.get_by_label("Mark").click();
    h.run();

    // The same windows carry on: still the same task, with the note in it.
    rig.at(Time(now.0 + 600));
    h.run();
    let timeline = h.state().timeline();
    assert_eq!(timeline.sessions.len(), 1);
    assert_eq!(timeline.sessions[0].active, before.sessions[0].active + 600);
    assert!(timeline.tasks[0].name.is_none());
    assert_eq!(timeline.marks.len(), 1);
    assert_eq!(timeline.marks[0].at, now);
    assert_eq!(timeline.sessions[0].marks, [0]);
    let line = format!("> {}  marked \"Budget\"", now.hm());
    assert!(h.query_by_label(&line).is_some());
    // It is in the log too, between the focus changes.
    assert!(h.query_by_label("marked \"Budget\"").is_some());

    // When the work does change around it, the marker counts. Open the row
    // to split it there.
    let title = timeline.sessions[0].label.headline.clone();
    h.get_all_by_label(&title).last().unwrap().click();
    h.run();
    h.get_by_label("split here").click();
    h.run();
    let split = h.state().timeline();
    assert_eq!(split.sessions.len(), 2);
    assert_eq!(split.sessions[1].start, now);
    assert_eq!(split.sessions[1].opened_by, Some(0));
    // The row it starts can take its name from it.
    let line = format!("> {}  marked \"Budget\"  (starts here)", now.hm());
    assert!(h.query_by_label(&line).is_some());

    // Undo takes back the split, then the marker.
    h.get_by_label("Undo").click();
    h.run();
    assert_eq!(h.state().timeline().sessions.len(), 1);
    h.get_by_label("Undo").click();
    h.run();
    assert!(h.state().timeline().marks.is_empty());
    assert!(h.state().journal().marks().is_empty());
}

#[test]
fn a_break_is_a_marker_too() {
    let (mut h, rig) = harness();
    rig.focus(t("09:00"), NOTEPAD, "report.txt - Notepad");
    rig.at(t("09:20"));
    h.run();
    h.get_by_label("Break").click();
    h.run();
    let marks = h.state().journal().marks();
    assert_eq!(marks.len(), 1);
    assert_eq!(marks[0].mark, Mark::Break(String::new()));
    assert_eq!(marks[0].at, t("09:20"));
    assert!(h.query_by_label("> 09:20  break").is_some());
}

#[test]
fn a_marked_day_replayed_shows_its_markers_with_their_rows() {
    let (mut h, rig) = harness();
    rig.play_marked(&scenarios::marked_day(), &scenarios::marked_day_marks());
    h.run();
    let timeline = h.state().timeline();
    assert_eq!(timeline.sessions.len(), 4);
    assert!(
        h.query_by_label("> 09:33  marked \"invoice export\"  (starts here)")
            .is_some()
    );
    assert!(
        h.query_by_label("> 09:53  marked \"Sam called about the export format\"")
            .is_some()
    );
    // Windows closed are in the log, not in the rows.
    assert!(
        h.query_by_label_contains("closed Search result 11")
            .is_some()
    );
}

#[test]
fn opening_a_row_shows_what_is_in_it_and_lets_it_be_taken_apart() {
    let (mut h, rig) = harness();
    let webmail = scenarios::ALL.iter().find(|s| s.name == "webmail").unwrap();
    rig.play(&(webmail.spans)());
    h.run();
    let timeline = h.state().timeline().clone();
    let (row, _) = timeline
        .sessions
        .iter()
        .enumerate()
        .find(|(_, s)| s.members.iter().any(|m| !m.doubts.is_empty()))
        .expect("a row with something doubtful in it");
    let title = timeline.sessions[row].label.headline.clone();
    // Closed rows say there is something to check, but not what.
    assert!(h.query_by_label("? 1").is_some());
    let member = "Mozilla Firefox · Gmail";
    assert!(h.query_by_label(member).is_none());

    // The last of these is the row (the first is the "now" line).
    h.get_all_by_label(&title).last().unwrap().click();
    h.run();
    assert!(h.query_by_label(member).is_some());
    assert!(
        h.query_by_label_contains("also a task of its own")
            .is_some()
    );

    // Taking it out "just here": hovering over that outlines the rows it
    // would change.
    let doubtful = timeline.sessions[row]
        .members
        .iter()
        .position(|m| !m.doubts.is_empty())
        .unwrap();
    h.get_all_by_label("own task…")
        .nth(doubtful)
        .unwrap()
        .click();
    h.run();
    h.get_by_label("just here").hover();
    h.run();
    assert!(h.state().previewed().contains(&row));

    h.get_by_label("just here").click();
    h.run();
    let after = h.state().timeline();
    assert!(after.sessions.len() > timeline.sessions.len());
    assert!(
        after
            .sessions
            .iter()
            .all(|s| s.members.iter().all(|m| m.doubts.is_empty())),
        "{}",
        timechopper::interpret::render::render(after, Default::default())
    );
}

#[test]
fn rows_can_be_named_and_confirmed() {
    let (mut h, rig) = harness();
    rig.play(&(scenarios::ALL[0].spans)());
    h.run();
    let title = h.state().timeline().sessions[0].label.headline.clone();
    // The last of these is the row (the first is the "now" line).
    h.get_all_by_label(&title).last().unwrap().click();
    h.run();
    h.get_by_label("Name…").click();
    h.run();
    // The name box takes the keyboard as soon as it opens.
    // (The first text box is the one for a new task.)
    let boxes: Vec<_> = h.get_all_by_role(Role::TextInput).collect();
    boxes[1].type_text("Q3 report");
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    let timeline = h.state().timeline();
    assert_eq!(timeline.tasks[0].name.as_deref(), Some("Q3 report"));

    h.get_by_label("Confirm").click();
    h.run();
    assert!(h.state().timeline().sessions[0].confirmed);
    assert!(h.query_by_label("confirmed").is_some());
}

#[test]
fn going_away_closes_the_row() {
    let (mut h, rig) = harness();
    rig.focus(t("09:00"), NOTEPAD, "report.txt - Notepad");
    rig.send
        .send(Record::Focus(Event {
            at: t("09:10"),
            window: None,
        }))
        .unwrap();
    rig.at(t("09:30"));
    h.run();
    let timeline = h.state().timeline();
    assert_eq!(timeline.tracked, 600);
    assert_eq!(h.state().current_session(), None);
    assert!(h.query_by_label("nothing in focus").is_some());
    assert!(h.query_by_label("09:00–09:10").is_some());
}

#[test]
fn corrections_are_kept_across_a_restart() {
    let dir = std::env::temp_dir().join(format!("timechopper-ui-{}", std::process::id()));
    let path = dir.join("day.log");
    let _ = std::fs::remove_dir_all(&dir);
    let webmail = scenarios::ALL.iter().find(|s| s.name == "webmail").unwrap();
    let spans = (webmail.spans)();
    let end = spans.last().unwrap().end;

    // A first run: the day happens, the user marks, names, takes mail out
    // and confirms a row.
    let (mut h, rig) = harness_logged(Some(&path), Time::hms(8, 0, 0));
    rig.play(&spans);
    h.run();
    h.state_mut().mark(Mark::Note("report".into()));
    let t = h.state().timeline().clone();
    let report = t.sessions.iter().position(|s| s.members.len() > 1).unwrap();
    let gmail = t.sessions[report]
        .members
        .iter()
        .position(|m| !m.doubts.is_empty())
        .unwrap();
    h.state_mut().apply(t.name(report, "Q3 report"));
    h.state_mut().apply(t.separate(report, gmail, Scope::Here));
    let t = h.state().timeline().clone();
    h.state_mut().apply(t.confirm(0)); // the earliest row
    h.run();
    let before = h.state().timeline().clone();
    assert_eq!(h.state().journal().hints().len(), 3);
    drop(h);

    // Started again later the same day: everything is as it was.
    let (mut h, _rig) = harness_logged(Some(&path), end);
    h.run();
    let after = h.state().timeline().clone();
    assert!(before.diff(&after).is_empty(), "{:?}", before.diff(&after));
    assert_eq!(after.marks.len(), 1);
    assert_eq!(h.state().journal().hints().len(), 3);
    assert!(
        after
            .tasks
            .iter()
            .any(|t| t.name.as_deref() == Some("Q3 report"))
    );
    assert!(h.query_by_label("confirmed").is_some());

    // Undo still works on what was done before the restart, and is kept
    // too.
    h.get_by_label("Undo").click();
    h.run();
    assert_eq!(h.state().journal().hints().len(), 2);
    drop(h);
    let (h, _rig) = harness_logged(Some(&path), end);
    assert_eq!(h.state().journal().hints().len(), 2);
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.matches(" @hint ").count(), 3);
    assert_eq!(text.matches(" @unhint").count(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn what_earlier_days_say_is_offered_then_applied() {
    use timechopper::interpret::history::Lessons;
    let days = scenarios::week();
    let learned = |n: usize| {
        let mut journal = Journal::new(Params::default());
        journal.learn(Lessons::learn(&days[..n], &Params::default()));
        journal
    };

    // Tuesday: Monday's name for the billing work is offered on its row.
    let (mut h, rig) = harness_with(learned(1), None, Time::hms(8, 0, 0));
    rig.play(&days[1].spans);
    h.run();
    assert!(h.query_by_label("? called \"Billing\" on Mon").is_some());
    assert!(h.state().timeline().tasks.iter().all(|t| t.name.is_none()));
    let uses: Vec<_> = h.get_all_by_label("use").collect();
    // Two offers: Billing, and Monday's ticket number.
    assert_eq!(uses.len(), 2);
    // The offer for Billing is on the older row: the last one listed.
    h.get_all_by_label("use").last().unwrap().click();
    h.run();
    let t = h.state().timeline();
    assert!(t.tasks.iter().any(|t| t.name.as_deref() == Some("Billing")));
    assert!(h.query_by_label("? called \"Billing\" on Mon").is_none());
    drop(h);

    // Thursday: applied without asking; "not this" takes it away.
    let (mut h, rig) = harness_with(learned(3), None, Time::hms(8, 0, 0));
    rig.play(&days[3].spans);
    h.run();
    let t = h.state().timeline().clone();
    let billing = t
        .sessions
        .iter()
        .position(|s| t.tasks[s.task].name.as_deref() == Some("Billing"))
        .unwrap();
    assert!(
        h.query_by_label_contains("own task, as on Mon, Tue, Wed")
            .is_some()
    );
    h.get_all_by_label("Billing").last().unwrap().click();
    h.run();
    assert!(h.query_by_label("named as on Mon, Tue, Wed").is_some());
    h.get_by_label("not this").click();
    h.run();
    let t = h.state().timeline();
    assert_eq!(t.tasks[t.sessions[billing].task].name, None);
}

#[test]
fn a_task_can_be_divided_into_parts_from_its_row() {
    let (mut h, rig) = harness();
    let brief = scenarios::ALL.iter().find(|s| s.name == "brief").unwrap();
    rig.play(&(brief.spans)());
    h.run();
    let t = h.state().timeline().clone();
    assert_eq!(t.sessions.len(), 2);
    let row = t
        .sessions
        .iter()
        .position(|s| s.members.iter().any(|m| m.activity.app == "winword"))
        .unwrap();
    let title = t.sessions[row].label.headline.clone();
    h.get_all_by_label(&title).last().unwrap().click();
    h.run();
    h.get_by_label("Divide by app").click();
    h.run();
    let t = h.state().timeline().clone();
    assert_eq!(t.sessions.len(), 2, "dividing makes no new rows");
    let parts = &t.sessions[row].parts;
    assert_eq!(parts.len(), 2);
    let word = format!("Word {}", fmt_duration(parts[0].secs));
    assert!(h.query_by_label(&word).is_some());

    // Click a part to rename it.
    h.get_by_label(&word).click();
    h.run();
    h.get_all_by_role(Role::TextInput)
        .nth(1)
        .unwrap()
        .type_text("Drafting");
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    let t = h.state().timeline().clone();
    assert_eq!(t.tasks[t.sessions[row].task].parts[0].name, "Drafting");

    // Combine the morning's research into it: one task, still two rows.
    h.get_by_label("Combine with…").click();
    h.run();
    let research = 1 - row;
    let label = format!(
        "{}  {}",
        t.sessions[research].start.hm(),
        t.sessions[research].label.headline
    );
    h.get_by_label(&label).click();
    h.run();
    let t = h.state().timeline();
    assert_eq!(t.tasks.len(), 1);
    assert_eq!(t.sessions.len(), 2);
}
