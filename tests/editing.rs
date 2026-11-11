//! Seeing what the guess swept in, and changing it: the calls a UI would
//! make, tried on synthetic logs.

mod common;

use common::check;
use timechopper::interpret::scenarios::{self, CODE, EXCEL, FIREFOX, Script, TEAMS, WORD};
use timechopper::interpret::{
    Doubt, Hint, Journal, Params, Pattern, PossibleSplit, Scope, Session, Span, Time, Timeline,
    Why, Window,
};

fn journal(spans: &[Span]) -> Journal {
    Journal::from_spans(spans.to_vec(), Params::default())
}

fn end(spans: &[Span]) -> Time {
    spans.last().unwrap().end
}

/// The timeline of a finished log, checked.
fn read(journal: &Journal, spans: &[Span]) -> Timeline {
    let timeline = journal.timeline(end(spans));
    check(spans, &timeline);
    timeline
}

/// The index of the session with this headline; there must be exactly one.
fn row(timeline: &Timeline, headline: &str) -> usize {
    let mut rows =
        (0..timeline.sessions.len()).filter(|&i| timeline.sessions[i].label.headline == headline);
    let row = rows.next().unwrap_or_else(|| panic!("no row {headline}"));
    assert_eq!(rows.next(), None, "several rows {headline}");
    row
}

fn member(session: &Session, name: &str) -> usize {
    let found = session.members.iter().position(|m| m.name == name);
    found.unwrap_or_else(|| panic!("no member {name}"))
}

fn doubted(timeline: &Timeline) -> Vec<&str> {
    let members = timeline.sessions.iter().flat_map(|s| &s.members);
    members
        .filter(|m| !m.doubts.is_empty())
        .map(|m| m.name.as_str())
        .collect()
}

#[test]
fn a_session_says_what_it_is_made_of_and_why() {
    let spans = scenarios::project_switch();
    let timeline = read(&journal(&spans), &spans);
    let session = &timeline.sessions[row(&timeline, "timechopper")];
    let members: Vec<(&str, usize, Why)> = session
        .members
        .iter()
        .map(|m| (m.name.as_str(), m.visits, m.why))
        .collect();
    assert_eq!(
        members,
        [
            ("Visual Studio Code · timechopper", 8, Why::Main),
            ("windowsterminal · PowerShell", 3, Why::Returned),
            ("Mozilla Firefox · Rust", 2, Why::Returned),
            ("Mozilla Firefox · Stack Overflow", 2, Why::Returned),
        ]
    );
    // Nothing here is out of the ordinary.
    assert!(doubted(&timeline).is_empty());
    assert!(timeline.sessions.iter().all(|s| s.splits.is_empty()));
}

#[test]
fn the_tidy_days_raise_no_doubts() {
    for scenario in &scenarios::ALL[..6] {
        let spans = (scenario.spans)();
        let timeline = read(&journal(&spans), &spans);
        assert!(doubted(&timeline).is_empty(), "{}", scenario.name);
        let splits = timeline.sessions.iter().flat_map(|s| &s.splits);
        assert_eq!(splits.count(), 0, "{}", scenario.name);
    }
}

#[test]
fn webmail_is_swept_in_flagged_and_can_be_pulled_out() {
    let spans = scenarios::webmail();
    let mut journal = journal(&spans);
    let before = read(&journal, &spans);
    assert_eq!(before.sessions.len(), 2);

    // The mail read mid-report is counted with the report, because the
    // report uses the same browser; but it is marked as doubtful, because
    // mail was also a task of its own earlier.
    let report = row(&before, "report.txt");
    let mail = member(&before.sessions[report], "Mozilla Firefox · Gmail");
    let swept = &before.sessions[report].members[mail];
    assert_eq!((swept.secs, swept.visits, swept.why), (40, 1, Why::SameApp));
    let own = Doubt::Elsewhere {
        tasks: 1,
        own: true,
    };
    assert_eq!(swept.doubts, [own]);
    assert_eq!(doubted(&before), ["Mozilla Firefox · Gmail"]);

    // Pulling it out of this row only: the preview shows the report row
    // replaced by a shorter one plus a carve-out, and nothing else touched.
    let hint = before.separate(report, mail, Scope::Here);
    let (after, diff) = journal.preview(hint.clone(), end(&spans));
    assert_eq!(diff.gone, [report]);
    assert_eq!(diff.new.len(), 2);
    assert_eq!(journal.hints().len(), 0);
    journal.apply(hint);
    let after_apply = read(&journal, &spans);
    assert!(after.diff(&after_apply).is_empty());

    let report = &after.sessions[row(&after, "report.txt")];
    assert_eq!(report.active, before.sessions[1].active - 40);
    assert_eq!(report.elsewhere(), 40);
    // It joins the morning's mail as the same task, and sits inside the
    // report as a carve-out.
    let gmail = after.tasks.iter().find(|t| t.label.headline == "Gmail");
    let gmail = gmail.unwrap();
    assert_eq!((gmail.sessions.len(), gmail.active), (2, 340));
    let carved = &after.sessions[gmail.sessions[1]];
    assert_eq!(
        after.sessions[carved.parent.unwrap()].label.headline,
        "report.txt"
    );
    assert!(doubted(&after).is_empty());

    // Saying it of mail everywhere comes to the same thing here.
    journal.undo();
    journal.apply(before.separate(report_row(&before), mail, Scope::Everywhere));
    let pattern = Pattern::parse("firefox:gmail").unwrap();
    assert_eq!(journal.hints(), [Hint::apart(pattern)]);
    assert!(after.diff(&read(&journal, &spans)).is_empty());
}

fn report_row(timeline: &Timeline) -> usize {
    row(timeline, "report.txt")
}

#[test]
fn email_looked_at_twice_is_flagged_and_can_be_kept_separate() {
    let spans = scenarios::email_twice();
    let mut journal = journal(&spans);
    let before = read(&journal, &spans);
    let work = row(&before, "timechopper");
    let inbox = member(&before.sessions[work], "Outlook · Inbox");
    let swept = &before.sessions[work].members[inbox];
    assert_eq!(
        (swept.secs, swept.visits, swept.why),
        (75, 2, Why::Returned)
    );
    assert!(matches!(
        swept.doubts[..],
        [Doubt::Elsewhere { own: true, .. }]
    ));

    journal.apply(before.separate(work, inbox, Scope::Everywhere));
    let after = read(&journal, &spans);
    let email = after.tasks.iter().find(|t| t.label.headline == "Inbox");
    let email = email.unwrap();
    assert_eq!((email.sessions.len(), email.active), (2, 360 + 75));
    // The two looks, minutes apart, are one session of it, running
    // alongside the programming.
    let looks = &after.sessions[email.sessions[1]];
    assert_eq!((looks.visits, looks.active), (2, 75));
    let work = row(&after, "timechopper");
    assert_eq!(looks.parent, Some(work));
    let work = &after.sessions[work];
    assert_eq!(work.active, before.sessions[1].active - 75);
    assert_eq!(work.elsewhere(), 75);
}

#[test]
fn a_buried_change_of_project_is_offered_as_a_split() {
    let spans = scenarios::buried_switch();
    let mut journal = journal(&spans);
    let before = read(&journal, &spans);
    assert_eq!(before.sessions.len(), 1);
    let boundary = spans.iter().find(|s| s.window.title.contains("billing"));
    let boundary = boundary.unwrap().start;
    let offered = PossibleSplit {
        at: boundary,
        before: "timechopper".to_string(),
        after: "billing".to_string(),
    };
    assert_eq!(before.sessions[0].splits, [offered]);

    // Accepting it.
    journal.apply(before.split(0, boundary).unwrap());
    let after = read(&journal, &spans);
    let rows: Vec<_> = after
        .sessions
        .iter()
        .map(|s| (s.label.headline.as_str(), s.start, s.end))
        .collect();
    let (start, end) = (spans[0].start, end(&spans));
    assert_eq!(
        rows,
        [("timechopper", start, boundary), ("billing", boundary, end)]
    );
    assert_eq!(after.tasks.len(), 2);
    assert!(after.sessions.iter().all(|s| s.splits.is_empty()));
}

#[test]
fn a_split_the_guess_nearly_made_is_offered() {
    // A document, then a spreadsheet, with twice as long spent in a browser
    // on pages that say nothing: a third of each half is its own, short of
    // the half that would have split it.
    let mut s = Script::at(9, 0);
    for (app, title) in [(WORD, "notes.docx - Word"), (EXCEL, "costs.xlsx - Excel")] {
        for _ in 0..4 {
            s.focus(app, title, 100)
                .focus(FIREFOX, "New Tab — Mozilla Firefox", 200);
        }
    }
    let spans = s.finish();
    let timeline = read(&journal(&spans), &spans);
    assert_eq!(timeline.sessions.len(), 1);
    let offered = PossibleSplit {
        at: spans[8].start,
        before: "notes.docx".to_string(),
        after: "costs.xlsx".to_string(),
    };
    assert_eq!(timeline.sessions[0].splits, [offered]);
}

#[test]
fn a_split_can_fall_anywhere_and_what_follows_goes_with_the_new_task() {
    // An hour in one window, a meeting, then the same window again. The
    // user says they changed tasks half an hour in, without changing window.
    let code = Window {
        app: CODE.to_string(),
        title: "main.rs - timechopper - Visual Studio Code".to_string(),
    };
    let teams = Window {
        app: TEAMS.to_string(),
        title: "Weekly sync | Microsoft Teams".to_string(),
    };
    let mut journal = Journal::new(Params::default());
    journal.observe(Time::hms(9, 0, 0), Some(code.clone()));
    let now = Time::hms(9, 40, 0);
    let timeline = journal.timeline(now);
    assert_eq!(timeline.sessions.len(), 1);
    assert_eq!(timeline.sessions[0].active, 40 * 60);

    let at = Time::hms(9, 30, 0);
    journal.apply(timeline.split(0, at).unwrap());
    let timeline = journal.timeline(now);
    check(&journal.spans(now), &timeline);
    let rows: Vec<_> = timeline
        .sessions
        .iter()
        .map(|s| (s.task, s.active))
        .collect();
    assert_eq!(rows, [(0, 30 * 60), (1, 10 * 60)]);

    // The day goes on: more of the same window, a meeting, and back to it.
    journal.observe(Time::hms(10, 0, 0), Some(teams));
    journal.observe(Time::hms(10, 40, 0), Some(code));
    let now = Time::hms(11, 0, 0);
    let timeline = journal.timeline(now);
    check(&journal.spans(now), &timeline);
    let rows: Vec<_> = timeline
        .sessions
        .iter()
        .map(|s| (s.task, s.active))
        .collect();
    // Everything after the split, including what came after the meeting, is
    // the second task.
    assert_eq!(
        rows,
        [(0, 30 * 60), (1, 30 * 60), (2, 40 * 60), (1, 20 * 60)]
    );
    assert_eq!(timeline.tasks[0].sessions.len(), 1);
    assert_eq!(timeline.tasks[1].sessions.len(), 2);
}

#[test]
fn a_split_takes_effect_when_the_row_next_has_focus() {
    let spans = scenarios::carve_out();
    let timeline = read(&journal(&spans), &spans);
    let (report, email) = (&timeline.sessions[0], &timeline.sessions[1]);
    // Asked to split the report during the email, the split lands on the
    // report's next moment in focus.
    let during_email = Time(email.start.0 + 10);
    assert_eq!(
        timeline.split(0, during_email),
        Some(Hint::Split { at: email.end })
    );
    // Nothing to split at the very start or after the end.
    assert_eq!(timeline.split(0, report.start), None);
    assert_eq!(timeline.split(0, report.end), None);

    let mut journal = journal(&spans);
    journal.apply(timeline.split(0, during_email).unwrap());
    let after = read(&journal, &spans);
    let rows: Vec<_> = after.sessions.iter().map(|s| (s.start, s.end)).collect();
    // The email is no longer inside anything: it sits between the halves.
    assert_eq!(
        rows,
        [
            (report.start, email.start),
            (email.start, email.end),
            (email.end, report.end)
        ]
    );
    assert!(after.sessions.iter().all(|s| s.parent.is_none()));
    assert_eq!(after.tasks.len(), 3);
}

#[test]
fn a_carve_out_can_be_folded_back_in() {
    let spans = scenarios::carve_out();
    let mut journal = journal(&spans);
    let before = read(&journal, &spans);
    let (report, email) = (row(&before, "report.txt"), row(&before, "Inbox"));
    journal.apply(before.join(email, report).unwrap());
    let after = read(&journal, &spans);
    assert_eq!(after.sessions.len(), 1);
    assert_eq!(after.sessions[0].active, after.tracked);
    assert_eq!(after.sessions[0].elsewhere(), 0);
    let folded = member(&after.sessions[0], "Outlook");
    assert_eq!(after.sessions[0].members[folded].why, Why::Once);
    // There is nothing to join a row to itself.
    assert_eq!(after.join(0, 0), None);
}

#[test]
fn joining_one_row_leaves_the_tasks_other_rows_alone() {
    let spans = scenarios::messy_day();
    let mut journal = journal(&spans);
    let before = read(&journal, &spans);
    let inbox = before.tasks.iter().find(|t| t.label.headline == "Inbox");
    let looks = inbox.unwrap().sessions.clone();
    assert_eq!(looks.len(), 4);
    // The first quick look at email was part of the programming after all.
    let look = looks[1];
    let work = before.sessions[look].parent.unwrap();
    let hint = before.join(look, work).unwrap();
    let (after, diff) = journal.preview(hint.clone(), end(&spans));
    assert_eq!(diff.gone, [work, look]);
    assert_eq!(diff.new.len(), 1);
    journal.apply(hint);

    let inbox = after.tasks.iter().find(|t| t.label.headline == "Inbox");
    assert_eq!(inbox.unwrap().sessions.len(), 3);
    let grown = &after.sessions[diff.new[0]];
    assert_eq!(grown.active, before.sessions[work].active + 30);
    check(&spans, &after);
}

#[test]
fn two_rows_can_be_made_one_task() {
    let spans = scenarios::project_switch();
    let mut journal = journal(&spans);
    let before = read(&journal, &spans);
    let (first, second) = (row(&before, "timechopper"), row(&before, "billing"));
    journal.apply(before.join(second, first).unwrap());
    let after = read(&journal, &spans);
    assert_eq!((after.tasks.len(), after.sessions.len()), (1, 1));
    assert_eq!(after.sessions[0].active, after.tracked);
    // Having been told, it doesn't suggest undoing it.
    assert!(after.sessions[0].splits.is_empty());
}

#[test]
fn names_stick_to_tasks_and_tie_them_together() {
    let spans = scenarios::resumed();
    let mut journal = journal(&spans);
    let before = read(&journal, &spans);
    journal.apply(before.name(0, "Budget"));
    let named = read(&journal, &spans);
    // Both sessions of the task carry the name; the meeting doesn't.
    let names: Vec<_> = named
        .sessions
        .iter()
        .map(|s| named.tasks[s.task].name.as_deref())
        .collect();
    assert_eq!(names, [Some("Budget"), None, Some("Budget")]);
    // Only the rows that were renamed count as changed.
    assert_eq!(before.diff(&named).gone, [0, 2]);

    // Calling the meeting the same thing makes it the same task, and with
    // nothing left between its parts, one row.
    journal.apply(named.name(1, "Budget"));
    let merged = read(&journal, &spans);
    assert_eq!((merged.tasks.len(), merged.sessions.len()), (1, 1));
    assert_eq!(merged.tasks[0].active, merged.tracked);
}

/// Feeds a journal what the watcher would have reported of a finished log
/// from `from` up to, but not including, `now`.
fn replay(journal: &mut Journal, spans: &[Span], from: Time, now: Time) {
    for (i, span) in spans.iter().enumerate() {
        if from <= span.start && span.start < now {
            journal.observe(span.start, Some(span.window.clone()));
        }
        // Time away shows up as nothing having focus.
        let away = spans.get(i + 1).is_none_or(|next| next.start > span.end);
        if away && from <= span.end && span.end < now {
            journal.observe(span.end, None);
        }
    }
}

#[test]
fn the_timeline_keeps_up_with_a_growing_log() {
    let spans = scenarios::messy_day();
    let mut journal = Journal::new(Params::default());
    let mut last = spans[0].start;
    // Look at the timeline a minute into every window of the day.
    for span in &spans {
        let now = Time(span.start.0 + 60).min(span.end);
        replay(&mut journal, &spans, last, now);
        last = now;
        let so_far = journal.spans(now);
        let timeline = journal.timeline(now);
        check(&so_far, &timeline);
        // The row being worked on right now runs up to the present.
        let current = timeline.sessions.iter().map(|s| s.end).max();
        assert_eq!(current, Some(now));
        assert_eq!(so_far.last().unwrap().end, now);
    }
    // Once the day is over, it reads the same as if read in one go.
    let now = end(&spans);
    replay(&mut journal, &spans, last, now);
    assert!(journal.spans(now) == spans);
    let whole = read(&self::journal(&spans), &spans);
    assert!(whole.diff(&journal.timeline(now)).is_empty());
}

#[test]
fn a_confirmed_row_stays_as_it_was() {
    let spans = scenarios::messy_day();
    let mut journal = Journal::new(Params::default());
    let half_past = Time::hms(9, 30, 0);
    replay(&mut journal, &spans, spans[0].start, half_past);

    // At 9:30 the terminal has been used once, so it looks like an
    // interruption of the programming. The user settles it: it was one.
    let early = journal.timeline(half_past);
    let terminal = row(&early, "PowerShell");
    assert!(early.sessions[terminal].parent.is_some());
    let kept = early.intervals(terminal);
    journal.apply(early.name(terminal, "Deploy"));
    journal.apply(journal.timeline(half_past).confirm(terminal));

    // By the end of the day the terminal has been used many times, and
    // every other visit to it is counted with the programming. The
    // confirmed one is still its own row.
    replay(&mut journal, &spans, half_past, end(&spans));
    let late = read(&journal, &spans);
    let deploy = late
        .tasks
        .iter()
        .find(|t| t.name.as_deref() == Some("Deploy"));
    let deploy = deploy.unwrap();
    assert_eq!(deploy.sessions.len(), 1);
    let session = deploy.sessions[0];
    assert!(late.sessions[session].confirmed);
    assert_eq!(late.intervals(session), kept);
    assert_eq!(late.sessions[session].members[0].why, Why::Confirmed);
    let others = late.sessions.iter().filter(|s| !s.confirmed);
    assert!(others.clone().all(|s| s.label.headline != "PowerShell"));

    // Without the confirmation, that row would be gone.
    let mut unconfirmed = journal.clone();
    unconfirmed.retract(1);
    let late = read(&unconfirmed, &spans);
    assert!(late.sessions.iter().all(|s| !s.confirmed));
    assert!(
        late.sessions
            .iter()
            .all(|s| s.label.headline != "PowerShell")
    );

    // Nor do later corrections disturb it.
    journal.apply(Hint::apart(Pattern::app("windowsterminal")));
    journal.apply(Hint::Split { at: kept[0].0 });
    let late = read(&journal, &spans);
    let confirmed: Vec<_> = (0..late.sessions.len())
        .filter(|&i| late.sessions[i].confirmed)
        .collect();
    assert_eq!(confirmed.len(), 1);
    assert_eq!(late.intervals(confirmed[0]), kept);
}

#[test]
fn corrections_can_be_taken_back() {
    let spans = scenarios::project_switch();
    let mut journal = journal(&spans);
    let original = read(&journal, &spans);
    journal.apply(original.join(1, 0).unwrap());
    journal.apply(original.name(0, "Everything"));
    assert_eq!(read(&journal, &spans).sessions.len(), 1);
    assert!(matches!(journal.retract(0), Hint::Join { .. }));
    assert_eq!(read(&journal, &spans).sessions.len(), 2);
    assert!(matches!(journal.undo(), Some(Hint::Name { .. })));
    assert_eq!(journal.undo(), None);
    assert!(original.diff(&read(&journal, &spans)).is_empty());
}
