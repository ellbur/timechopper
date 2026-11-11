//! Dividing a task into parts (drafting and research) without cutting it
//! into pieces: parts are shares of the task's time, wherever it fell, and
//! the rows stay as they were.

mod common;

use common::check;
use timechopper::interpret::history::{Day, Lessons};
use timechopper::interpret::scenarios::brief;
use timechopper::interpret::{Hint, Journal, OTHER, Params, Part, Span, Time, Timeline, interpret};

fn read(spans: &[Span], hints: &[Hint]) -> Timeline {
    let timeline = interpret(spans, hints, &Params::default());
    check(spans, &timeline);
    timeline
}

/// Rows as (start, end, active), which dividing must leave alone.
fn rows(timeline: &Timeline) -> Vec<(Time, Time, u64)> {
    timeline
        .sessions
        .iter()
        .map(|s| (s.start, s.end, s.active))
        .collect()
}

fn part(parts: &[Part], name: &str) -> u64 {
    parts
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no part {name} in {parts:?}"))
        .secs
}

/// The drafting row: the one with Word in it.
fn drafting(t: &Timeline) -> usize {
    t.sessions
        .iter()
        .position(|s| s.members.iter().any(|m| m.activity.app == "winword"))
        .unwrap()
}

#[test]
fn dividing_by_app_leaves_the_rows_alone() {
    let spans = brief();
    let plain = read(&spans, &[]);
    let row = drafting(&plain);
    assert!(plain.tasks[plain.sessions[row].task].parts.is_empty());

    let divide = plain.divide_by_app(row).unwrap();
    let divided = read(&spans, &[divide]);
    assert_eq!(rows(&divided), rows(&plain));
    let task = &divided.tasks[divided.sessions[row].task];
    assert_eq!(task.parts.len(), 2);
    let session = &divided.sessions[row];
    let word: u64 = spans
        .iter()
        .filter(|s| s.window.app == "WINWORD.EXE")
        .map(Span::secs)
        .sum();
    assert_eq!(part(&session.parts, "Word"), word);
    assert_eq!(
        part(&session.parts, "Mozilla Firefox"),
        session.active - word
    );
    // Each thing in the row says which part it counts towards.
    for member in &session.members {
        assert!(member.part.is_some());
    }
}

#[test]
fn parts_can_be_renamed_rearranged_and_undone() {
    let spans = brief();
    let plain = read(&spans, &[]);
    let row = drafting(&plain);
    let mut hints = vec![plain.divide_by_app(row).unwrap()];
    let t1 = read(&spans, &hints);
    hints.push(t1.rename_part(row, "Word", "Drafting"));
    let t2 = read(&spans, &hints);
    hints.push(t2.rename_part(row, "Mozilla Firefox", "Research"));
    let t3 = read(&spans, &hints);
    let task = &t3.tasks[t3.sessions[row].task];
    let names: Vec<&str> = task.parts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Drafting", "Research"]);

    // Moving the browser into drafting leaves one part with everything.
    let browser = t3.sessions[row]
        .members
        .iter()
        .position(|m| m.activity.app == "firefox")
        .unwrap();
    let moved = read(
        &spans,
        &[
            hints.clone(),
            vec![t3.move_to_part(row, browser, "Drafting")],
        ]
        .concat(),
    );
    let session = &moved.sessions[row];
    assert_eq!(part(&session.parts, "Drafting"), session.active);

    // A division that leaves something out counts it as "other".
    let word_only = Hint::Parts {
        at: t3.anchor(row),
        parts: vec![(
            timechopper::interpret::Pattern::app("winword"),
            "Drafting".into(),
        )],
    };
    let partial = read(&spans, &[word_only]);
    assert!(part(&partial.sessions[row].parts, OTHER) > 0);

    // And undividing takes it all back.
    hints.push(t3.undivide(row));
    let undone = read(&spans, &hints);
    assert!(undone.tasks.iter().all(|t| t.parts.is_empty()));
    assert_eq!(rows(&undone), rows(&plain));
}

#[test]
fn two_tasks_combined_keep_their_time_as_parts() {
    let spans = brief();
    let plain = read(&spans, &[]);
    assert_eq!(plain.tasks.len(), 2);
    let row = drafting(&plain);
    let research = 1 - row;
    let combine = plain.combine(research, row).unwrap();
    let combined = read(&spans, std::slice::from_ref(&combine));
    // One task, still two rows.
    assert_eq!(combined.tasks.len(), 1);
    assert_eq!(rows(&combined), rows(&plain));
    let task = &combined.tasks[0];
    assert_eq!(
        part(&task.parts, "Westlaw"),
        plain.sessions[research].active
    );
    assert_eq!(
        part(&task.parts, "Smith v. Jones brief.docx"),
        plain.sessions[row].active
    );

    // Then divided by app: the morning's research and the trips back to it
    // during the drafting are one part.
    let divide = combined.divide_by_app(row).unwrap();
    let divided = read(&spans, &[combine, divide]);
    let task = &divided.tasks[0];
    let word: u64 = spans
        .iter()
        .filter(|s| s.window.app == "WINWORD.EXE")
        .map(Span::secs)
        .sum();
    assert_eq!(part(&task.parts, "Word"), word);
    assert_eq!(part(&task.parts, "Mozilla Firefox"), task.active - word);
}

#[test]
fn a_division_is_carried_over_from_earlier_days() {
    // Each day the same brief, divided into drafting and research.
    let spans = brief();
    let plain = read(&spans, &[]);
    let row = drafting(&plain);
    let division = Hint::Parts {
        at: plain.anchor(row),
        parts: vec![
            (
                timechopper::interpret::Pattern::app("winword"),
                "Drafting".into(),
            ),
            (
                timechopper::interpret::Pattern::app("firefox"),
                "Research".into(),
            ),
        ],
    };
    let day = |label: &str, hints: Vec<Hint>| Day {
        label: label.into(),
        spans: spans.clone(),
        marks: Vec::new(),
        hints,
    };
    let mut days = vec![
        day("Mon", vec![division.clone()]),
        day("Tue", vec![division.clone()]),
        day("Wed", vec![division.clone()]),
        day("Thu", Vec::new()),
        day("Fri", Vec::new()),
    ];
    let view = |days: &[Day], n: usize| {
        let mut journal = Journal::from_spans(spans.clone(), Params::default());
        journal.learn(Lessons::learn(&days[..n], &Params::default()));
        for hint in &days[n].hints {
            journal.apply(hint.clone());
        }
        let timeline = journal.timeline(spans.last().unwrap().end);
        check(&spans, &timeline);
        timeline
    };

    // Tuesday, before dividing: offered.
    let mut tue = days[1].clone();
    tue.hints.clear();
    let first = view(&[days[0].clone(), tue], 1);
    let task = &first.tasks[first.sessions[row].task];
    assert!(task.parts.is_empty());
    let offer = task.division.as_ref().unwrap();
    assert!(!offer.applied);
    assert_eq!(offer.parts.len(), 2);

    // Thursday: divided without asking.
    let thu = view(&days, 3);
    let task = &thu.tasks[thu.sessions[row].task];
    let names: Vec<&str> = task.parts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Drafting", "Research"]);
    assert!(task.division.as_ref().unwrap().applied);

    // Undone on Thursday: Friday is not divided.
    days[3].hints = vec![thu.undivide(row)];
    let fri = view(&days, 4);
    assert!(fri.tasks.iter().all(|t| t.parts.is_empty()));
}
