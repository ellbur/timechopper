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
use timechopper::interpret::scenarios::{self, FIREFOX, NOTEPAD, OUTLOOK};
use timechopper::interpret::{Journal, Params, Span, Time, Window};
use timechopper::live::{self, FnClock, Observation};
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
            .send(Observation {
                at: t,
                window: Some(window),
            })
            .unwrap();
        self.at(t);
    }

    /// Feeds a whole log, leaving the clock at its end with its last
    /// window still in focus.
    fn play(&self, spans: &[Span]) {
        let mut events = live::events(spans);
        let end = events.pop().unwrap().at;
        for event in events {
            self.at(event.at);
            self.send.send(event).unwrap();
        }
        self.at(end);
    }
}

fn harness() -> (Harness<'static, Live>, Rig) {
    let (send, feed) = mpsc::channel();
    let clock = Arc::new(AtomicU64::new(Time::hms(9, 0, 0).0));
    let reading = clock.clone();
    let clock_fn = FnClock(move || Time(reading.load(Ordering::SeqCst)));
    let live = Live::new(
        Journal::new(Params::default()),
        feed,
        Arc::new(clock_fn),
        None,
    );
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
fn new_task_starts_from_now_and_leaves_the_morning_alone() {
    let (mut h, rig) = harness();
    rig.play(&(scenarios::ALL[0].spans)());
    h.run();
    let before = h.state().timeline().clone();
    let now = h.state().timeline().sessions[0].end;

    h.get_by_role(Role::TextInput).click();
    h.run();
    h.get_by_role(Role::TextInput).type_text("Budget");
    h.run();
    h.get_by_label("New task").click();
    h.run();

    // Carry on in the same windows: that is now the new task.
    let later = Time(now.0 + 600);
    rig.at(later);
    h.run();
    let timeline = h.state().timeline();
    let current = h.state().current_session().unwrap();
    let task = &timeline.tasks[timeline.sessions[current].task];
    assert_eq!(task.name.as_deref(), Some("Budget"));
    assert_eq!(timeline.sessions[current].start, now);
    assert_eq!(timeline.sessions[current].active, 600);
    // What came before is untouched.
    assert_eq!(timeline.sessions[0].start, before.sessions[0].start);
    assert_eq!(timeline.sessions[0].active, before.sessions[0].active);
    assert!(h.query_all_by_label("Budget").count() >= 2); // now, and the row

    // Undo takes it back.
    h.get_by_label("Undo").click();
    h.run();
    h.get_by_label("Undo").click();
    h.run();
    assert_eq!(h.state().timeline().sessions.len(), 1);
    assert!(h.state().journal().hints().is_empty());
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
    h.get_by_label("OK").click();
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
        .send(Observation {
            at: t("09:10"),
            window: None,
        })
        .unwrap();
    rig.at(t("09:30"));
    h.run();
    let timeline = h.state().timeline();
    assert_eq!(timeline.tracked, 600);
    assert_eq!(h.state().current_session(), None);
    assert!(h.query_by_label("nothing in focus").is_some());
    assert!(h.query_by_label("09:00–09:10").is_some());
}
