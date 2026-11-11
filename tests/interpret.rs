//! The interpretation, tried on synthetic working days.

mod common;

use common::check;
use timechopper::interpret::render::{Detail, render};
use timechopper::interpret::scenarios::{self, CODE, FIREFOX, NOTEPAD, OUTLOOK, Script, TERMINAL};
use timechopper::interpret::{
    Hint, Lens, Mark, Marker, Params, Pattern, Span, Time, Timeline, interpret, interpret_marked,
};

fn guess(spans: &[Span]) -> Timeline {
    read(spans, &[])
}

fn read(spans: &[Span], hints: &[Hint]) -> Timeline {
    let timeline = interpret(spans, hints, &Params::default());
    check(spans, &timeline);
    timeline
}

fn secs_in(spans: &[Span], app: &str) -> u64 {
    let of_app = spans.iter().filter(|s| s.window.app == app);
    of_app.map(Span::secs).sum()
}

#[test]
fn alternation_collapses_into_one_task() {
    let spans = scenarios::alternation();
    let timeline = guess(&spans);
    assert_eq!(timeline.sessions.len(), 1);
    let session = &timeline.sessions[0];
    assert_eq!(session.visits, 9);
    assert_eq!(session.active, timeline.tracked);
    assert_eq!(session.elsewhere(), 0);
    assert_eq!(session.label.headline, "report.txt");
    assert_eq!(session.label.apps, ["Notepad", "Mozilla Firefox"]);
    // What was looked up is still there to jog the memory.
    assert_eq!(
        session.label.also[0].text,
        "How to structure a quarterly report"
    );
}

#[test]
fn email_is_carved_out_without_splitting_the_task() {
    let plain = guess(&scenarios::alternation());
    let spans = scenarios::carve_out();
    let timeline = guess(&spans);
    assert_eq!(timeline.sessions.len(), 2);

    // The report is still one session, and no longer or shorter for having
    // been interrupted.
    let report = &timeline.sessions[0];
    assert_eq!(report.label.headline, "report.txt");
    assert_eq!(report.active, plain.sessions[0].active);
    assert_eq!(report.visits, plain.sessions[0].visits);
    assert_eq!(report.elsewhere(), 40);
    assert_eq!(report.parent, None);

    let email = &timeline.sessions[1];
    assert_eq!(email.label.headline, "Inbox");
    assert_eq!(email.label.apps, ["Outlook"]);
    assert_eq!(email.active, 40);
    assert_eq!(email.parent, Some(0));
    assert_ne!(email.task, report.task);
}

#[test]
fn hint_makes_an_alternation_two_parallel_tasks() {
    let spans = scenarios::carve_out();
    let timeline = read(&spans, &[Hint::apart(Pattern::app("notepad"))]);
    assert_eq!(timeline.tasks.len(), 3);
    assert_eq!(timeline.sessions.len(), 3);

    // Each is one session running the length of the alternation, with the
    // other's time (and the email's) carved out of it.
    let notepad = &timeline.sessions[0];
    assert_eq!(notepad.label.apps, ["Notepad"]);
    assert_eq!(notepad.active, secs_in(&spans, NOTEPAD));
    assert_eq!(
        notepad.elsewhere(),
        secs_in(&spans, FIREFOX) + secs_in(&spans, OUTLOOK)
    );
    let browser = &timeline.sessions[1];
    assert_eq!(browser.label.apps, ["Mozilla Firefox"]);
    assert_eq!(browser.active, secs_in(&spans, FIREFOX));
    assert_eq!(browser.visits, 4);
    assert!(browser.elsewhere() > 0);
}

#[test]
fn an_uninformative_window_is_explained_by_its_companion() {
    let timeline = guess(&scenarios::uninformative());
    assert_eq!(timeline.sessions.len(), 1);
    let label = &timeline.sessions[0].label;
    assert_eq!(label.headline, "Kubernetes ingress timeout");
    assert!(label.apps.contains(&"Notepad".to_string()));
    assert!(label.also.iter().all(|e| e.text != "Untitled"));
}

#[test]
fn a_change_of_project_is_a_change_of_task() {
    let spans = scenarios::project_switch();
    let timeline = guess(&spans);
    let headlines: Vec<_> = timeline
        .sessions
        .iter()
        .map(|s| s.label.headline.as_str())
        .collect();
    assert_eq!(headlines, ["timechopper", "billing"]);
    // The terminal and the browser are used throughout, yet each visit is
    // counted with the project it was made for.
    assert_eq!(timeline.sessions[0].label.apps.len(), 3);
    assert_eq!(timeline.sessions[1].label.apps.len(), 3);
    let boundary = spans
        .iter()
        .find(|s| s.window.title.contains("billing"))
        .unwrap();
    assert_eq!(timeline.sessions[0].end, boundary.start);
    assert_eq!(timeline.sessions[1].start, boundary.start);
}

#[test]
fn hints_can_merge_what_the_guess_split() {
    let spans = scenarios::project_switch();
    let together = Hint::Together(Pattern::title("timechopper"), Pattern::title("billing"));
    assert_eq!(read(&spans, &[together]).sessions.len(), 1);

    // Or by declaring that this editor's titles don't matter.
    let lens = Hint::Lens {
        app: CODE.to_string(),
        lens: Lens::App,
    };
    assert_eq!(read(&spans, &[lens]).sessions.len(), 1);
}

#[test]
fn drift_needs_the_shared_app_to_be_minor() {
    // Two documents, one after the other, each with trips to a browser on
    // pages that say nothing.
    let day = |editing: u64, browsing: u64| {
        let mut s = Script::at(9, 0);
        for file in ["a.txt - Notepad", "b.txt - Notepad"] {
            for _ in 0..4 {
                s.focus(NOTEPAD, file, editing).focus(
                    FIREFOX,
                    "New Tab — Mozilla Firefox",
                    browsing,
                );
            }
        }
        s.finish()
    };
    // Mostly editing: the change of document is a change of task.
    assert_eq!(guess(&day(300, 60)).tasks.len(), 2);
    // Mostly browsing: nothing says the documents aren't incidental.
    assert_eq!(guess(&day(60, 300)).tasks.len(), 1);
}

#[test]
fn a_long_absence_makes_a_new_session_of_the_same_task() {
    let spans = scenarios::resumed();
    let timeline = guess(&spans);
    assert_eq!(timeline.tasks.len(), 2);
    assert_eq!(timeline.sessions.len(), 3);
    let [before, meeting, after] = &timeline.sessions[..] else {
        unreachable!()
    };
    assert_eq!(before.task, after.task);
    assert_eq!(meeting.label.headline, "Weekly sync");
    assert_eq!(meeting.active, 40 * 60);
    assert_eq!(meeting.parent, None);
    let budget = &timeline.tasks[before.task];
    assert_eq!(budget.label.headline, "budget-2027.xlsx");
    assert_eq!(budget.active, before.active + after.active);

    // With a more generous idea of an interruption, the meeting is one.
    let params = Params {
        max_gap: 60 * 60,
        ..Params::default()
    };
    let timeline = interpret(&spans, &[], &params);
    check(&spans, &timeline);
    assert_eq!(timeline.sessions.len(), 2);
    assert_eq!(timeline.sessions[1].parent, Some(0));
    assert_eq!(timeline.sessions[0].elsewhere(), 40 * 60);
}

#[test]
fn a_messy_day() {
    let spans = scenarios::messy_day();
    let timeline = guess(&spans);
    let mut tasks: Vec<(&str, usize)> = timeline
        .tasks
        .iter()
        .map(|t| (t.label.headline.as_str(), t.sessions.len()))
        .collect();
    tasks.sort();
    assert_eq!(
        tasks,
        [
            ("Exchange rates", 1),
            ("Inbox", 4),
            ("Q3 report.docx", 1),
            ("Weekly sync", 1),
            ("billing", 1),
            ("timechopper", 2),
        ]
    );
    // The two quick looks at email sit inside the programming sessions.
    let inbox = timeline
        .tasks
        .iter()
        .find(|t| t.label.headline == "Inbox")
        .unwrap();
    let parents: Vec<Option<&str>> = inbox
        .sessions
        .iter()
        .map(|&s| timeline.sessions[s].parent)
        .map(|p| p.map(|p| timeline.sessions[p].label.headline.as_str()))
        .collect();
    assert_eq!(
        parents,
        [None, Some("timechopper"), Some("timechopper"), None]
    );
    assert_eq!(inbox.active, secs_in(&spans, OUTLOOK));
    // Lunch belongs to nothing.
    let extent = spans.last().unwrap().end.0 - spans[0].start.0;
    assert_eq!(timeline.tracked, extent - 45 * 60);

    let text = render(&timeline, Detail::Doubts);
    assert!(text.contains("part 2 of 2"), "{text}");
    assert!(text.contains("  tracked\n"), "{text}");
}

#[test]
fn a_task_of_its_own_runs_alongside_everything() {
    // The terminal, declared its own task, runs alongside the programming
    // instead of being part of it.
    let spans = scenarios::messy_day();
    let timeline = read(&spans, &[Hint::apart(Pattern::app(TERMINAL))]);
    let terminal = timeline
        .tasks
        .iter()
        .find(|t| t.label.apps == ["windowsterminal"])
        .unwrap();
    assert_eq!(terminal.active, secs_in(&spans, TERMINAL));
    // Ten visits, in four stretches separated by long absences.
    assert_eq!(terminal.sessions.len(), 4);
    for &s in &terminal.sessions {
        let parent = timeline.sessions[s].parent.unwrap();
        let parent = &timeline.sessions[parent].label.headline;
        assert!(parent == "timechopper" || parent == "billing");
    }
    // The email that interrupted the programming is carved out of that, not
    // of the terminal session that happens to span it more tightly.
    let email = timeline
        .sessions
        .iter()
        .find(|s| s.label.headline == "Inbox" && s.active == 45)
        .unwrap();
    let parent = &timeline.sessions[email.parent.unwrap()];
    assert_eq!(parent.label.headline, "timechopper");
}

/// A small deterministic source of arbitrary choices.
struct Dice(u64);

impl Dice {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }

    fn pick<T: Clone>(&mut self, from: &[T]) -> T {
        from[self.below(from.len() as u64) as usize].clone()
    }

    /// Any moment of the day, whether or not anything was in focus.
    fn moment(&mut self) -> Time {
        Time(8 * 3600 + self.below(6 * 3600))
    }

    fn range(&mut self) -> (Time, Time) {
        let from = self.moment();
        (from, Time(from.0 + self.below(3600)))
    }
}

/// Times add up for any log and any hints, not just the tidy ones.
#[test]
fn every_second_is_counted_once() {
    let apps = [NOTEPAD, FIREFOX, OUTLOOK, CODE, TERMINAL];
    let titles = ["a - X", "b - X", "c | d - Notepad", "Untitled", "", "a — b"];
    let mut dice = Dice(42);
    for round in 0..400 {
        let mut script = Script::at(8, 0);
        for _ in 0..dice.below(60) {
            let secs = match dice.below(10) {
                0 => 1200 + dice.below(3600),
                _ => 1 + dice.below(400),
            };
            match dice.below(12) {
                0 => script.idle(secs),
                _ => script.focus(dice.pick(&apps), dice.pick(&titles), secs),
            };
        }
        let spans = script.finish();
        let mut hints = Vec::new();
        for _ in 0..dice.below(6) {
            hints.push(match dice.below(11) {
                8 => Hint::Combine {
                    from: dice.moment(),
                    into: dice.moment(),
                },
                9 => Hint::Parts {
                    at: dice.moment(),
                    parts: vec![
                        (Pattern::app(dice.pick(&apps)), "p".into()),
                        (Pattern::title("a"), dice.pick(&["q", "p"]).to_string()),
                    ],
                },
                10 => Hint::Parts {
                    at: dice.moment(),
                    parts: Vec::new(),
                },
                0 => Hint::apart(Pattern::app(dice.pick(&apps))),
                1 => Hint::Apart {
                    pattern: Pattern::title("b"),
                    during: vec![dice.range(), dice.range()],
                },
                2 => Hint::Together(Pattern::app(NOTEPAD), Pattern::title("a")),
                3 => Hint::Lens {
                    app: CODE.to_string(),
                    lens: dice.pick(&[Lens::Title, Lens::App]),
                },
                4 => Hint::Split { at: dice.moment() },
                5 => Hint::Join {
                    from: dice.moment(),
                    into: dice.moment(),
                },
                6 => Hint::Name {
                    at: dice.moment(),
                    name: dice.pick(&["x", "y"]).to_string(),
                },
                _ => Hint::Fixed {
                    name: dice.pick(&[None, Some("x".to_string())]),
                    intervals: vec![dice.range(), dice.range()],
                },
            });
        }
        let params = Params {
            max_gap: [60, 900, 7200][round % 3],
            min_exclusive_share: [0.5, 0.2][round % 2],
            near_miss_share: [0.25, 0.0][round % 2],
            mark_tolerance: [0, 120, 600][round % 3],
        };
        let marks: Vec<Marker> = (0..round % 5)
            .map(|k| Marker {
                at: dice.range().0,
                mark: match (k % 3, spans.is_empty()) {
                    (0, _) | (2, true) => Mark::Note(format!("note {k}")),
                    (1, _) => Mark::Break(String::new()),
                    _ => Mark::Closed(spans[k % spans.len()].window.clone()),
                },
            })
            .collect();
        let timeline = interpret_marked(&spans, &marks, &hints, &params);
        check(&spans, &timeline);
        render(&timeline, Detail::Contents);
    }
}
