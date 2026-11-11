//! Markers: what the user noted along the way, and windows opened and
//! closed. They are evidence of where a task may have ended, weighed by the
//! guess, and kept with the rows for the user to reconstruct the day from.
//! They never stand in for a task.

mod common;

use common::check;
use timechopper::interpret::scenarios::{self, FIREFOX, NOTEPAD, Script};
use timechopper::interpret::{
    Hint, Journal, Mark, Marker, Params, Record, Span, Time, Timeline, interpret, interpret_marked,
};

fn read(spans: &[Span], marks: &[Marker], hints: &[Hint]) -> Timeline {
    let timeline = interpret_marked(spans, marks, hints, &Params::default());
    check(spans, &timeline);
    timeline
}

fn note(at: Time, text: &str) -> Marker {
    Marker {
        at,
        mark: Mark::Note(text.into()),
    }
}

fn t(text: &str) -> Time {
    Time::parse(text).unwrap()
}

/// The rows, as (start, end) pairs.
fn rows(timeline: &Timeline) -> Vec<(Time, Time)> {
    timeline.sessions.iter().map(|s| (s.start, s.end)).collect()
}

#[test]
fn a_marker_where_the_work_changes_somewhat_decides_it() {
    let spans = scenarios::buried_switch();
    // Unmarked, the change of project is only offered.
    let plain = read(&spans, &[], &[]);
    assert_eq!(plain.sessions.len(), 1);
    assert_eq!(plain.sessions[0].splits.len(), 1);
    let switch = plain.sessions[0].splits[0].at;
    assert_eq!(switch, t("09:33:20"));

    // Noted a little before getting to it: two rows, the second starting
    // where the work changed (not where the note was made), opened by it.
    let marked = read(&spans, &[note(t("09:32:40"), "billing")], &[]);
    assert_eq!(
        rows(&marked),
        [(t("09:00"), switch), (switch, t("10:06:40"))]
    );
    let second = &marked.sessions[1];
    assert_eq!(
        marked.marks[second.opened_by.unwrap()].mark,
        Mark::Note("billing".into())
    );
    assert!(second.splits.is_empty());

    // A window closed there says the same.
    let closed = Marker {
        at: t("09:33:20"),
        mark: Mark::Closed(spans[8].window.clone()),
    };
    assert_eq!(read(&spans, &[closed], &[]).sessions.len(), 2);

    // A window opened there doesn't: new windows open all the time.
    let opened = Marker {
        at: t("09:33:20"),
        mark: Mark::Opened(spans[10].window.clone()),
    };
    assert_eq!(read(&spans, &[opened], &[]).sessions.len(), 1);

    // Too far from the change to be about it: offered, as before.
    let far = read(&spans, &[note(t("09:20"), "billing")], &[]);
    assert_eq!(far.sessions.len(), 1);
    assert_eq!(far.sessions[0].marks.len(), 1);
}

#[test]
fn a_marker_where_nothing_changes_cuts_nothing() {
    let spans = (scenarios::ALL[0].spans)(); // alternation
    let plain = interpret(&spans, &[], &Params::default());
    for at in ["09:00", "09:05", "09:11:30", "09:20"] {
        let marked = read(&spans, &[note(t(at), "something new")], &[]);
        assert_eq!(rows(&marked), rows(&plain), "{at}");
        let session = &marked.sessions[0];
        let attached = session.marks.len() + usize::from(session.opened_by.is_some());
        assert_eq!(attached, 1, "{at}");
    }
}

#[test]
fn a_cut_made_by_a_marker_can_be_undone() {
    let spans = scenarios::buried_switch();
    let marks = [note(t("09:33"), "billing")];
    let marked = read(&spans, &marks, &[]);
    assert_eq!(marked.sessions.len(), 2);
    let join = marked.join(1, 0).unwrap();
    let joined = read(&spans, &marks, &[join]);
    assert_eq!(joined.sessions.len(), 1);
    // And it isn't offered again.
    assert!(joined.sessions[0].splits.is_empty());
    // The note is still there, inside the row.
    assert_eq!(joined.sessions[0].marks.len(), 1);
}

#[test]
fn a_marker_partway_through_a_row_can_be_split_at() {
    let spans = scenarios::marked_day();
    let marks = scenarios::marked_day_marks();
    let timeline = read(&spans, &marks, &[]);
    // "Sam called" and the break fall inside the billing row; neither cut it.
    let billing = timeline
        .sessions
        .iter()
        .position(|s| {
            s.opened_by
                .is_some_and(|k| timeline.marks[k].mark.text() == Some("invoice export"))
        })
        .unwrap();
    let inside: Vec<&Mark> = timeline.sessions[billing]
        .marks
        .iter()
        .map(|&k| &timeline.marks[k].mark)
        .collect();
    assert_eq!(
        inside,
        [
            &Mark::Note("Sam called about the export format".into()),
            &Mark::Break("lunch".into())
        ]
    );
    // Splitting at the break takes the browsing after it out of the row.
    let lunch = timeline.marks[timeline.sessions[billing].marks[1]].at;
    let split = timeline.split(billing, lunch).unwrap();
    let after = read(&spans, &marks, &[split]);
    let rest = after
        .sessions
        .iter()
        .find(|s| {
            s.opened_by
                .is_some_and(|k| after.marks[k].mark == Mark::Break("lunch".into()))
        })
        .expect("a row starting at the break");
    assert_eq!(rest.start, lunch);
    assert_eq!(rest.active, 240);
}

#[test]
fn markers_belong_to_the_row_they_announce() {
    // Mark, then switch a minute later to something new: the mark goes
    // with the new row, not the one in focus when it was made.
    let mut s = Script::at(9, 0);
    s.focus(NOTEPAD, "a.txt - Notepad", 600)
        .shift(-60)
        .note("reading up")
        .shift(60)
        .focus(FIREFOX, "Docs — Mozilla Firefox", 600)
        .focus(FIREFOX, "More docs — Mozilla Firefox", 600);
    let marks = s.marks();
    let spans = s.finish();
    let timeline = read(&spans, &marks, &[]);
    assert_eq!(timeline.sessions.len(), 2);
    assert!(timeline.sessions[0].opened_by.is_none());
    assert_eq!(timeline.sessions[1].opened_by, Some(0));
}

#[test]
fn markers_can_be_taken_back_and_survive_a_restart() {
    let spans = scenarios::buried_switch();
    let mut journal = Journal::from_spans(spans.clone(), Params::default());
    let end = spans.last().unwrap().end;
    journal.record(Record::Mark(note(t("09:33"), "billing")));
    assert_eq!(journal.timeline(end).sessions.len(), 2);
    journal.record(Record::Unmark {
        at: t("10:00"),
        mark: t("09:33"),
    });
    assert_eq!(journal.timeline(end).sessions.len(), 1);
    assert!(journal.marks().is_empty());
    // Only the user's markers can be taken back.
    let closed = Marker {
        at: t("09:40"),
        mark: Mark::Closed(spans[0].window.clone()),
    };
    journal.mark(closed);
    assert!(journal.unmark(t("09:40")).is_none());
    // Markers made later than "now" are not part of the day yet.
    journal.mark(note(t("09:33"), "billing"));
    assert_eq!(journal.timeline(t("09:30")).marks.len(), 0);
}
