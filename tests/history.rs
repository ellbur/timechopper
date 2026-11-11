//! Rules carried over from earlier days: offered at first, applied once they
//! have held for a few days, dropped when the user goes against them, and
//! always below the day's own corrections.

mod common;

use common::check;
use timechopper::interpret::history::{APPLY_AFTER, Day, Lessons};
use timechopper::interpret::scenarios::week;
use timechopper::interpret::{Hint, Journal, Params, Record, Time, Timeline};

/// Day `n` of `days`, with what the days before it teach and `hints` of its
/// own, checked.
fn view(days: &[Day], n: usize, hints: &[Hint]) -> Timeline {
    let mut journal = Journal::from_spans(days[n].spans.clone(), Params::default());
    journal.learn(Lessons::learn(&days[..n], &Params::default()));
    for hint in hints {
        journal.apply(hint.clone());
    }
    let end = days[n].spans.last().unwrap().end;
    let timeline = journal.timeline(end);
    check(&days[n].spans, &timeline);
    timeline
}

/// The task identified by this subject ("billing", "Helpdesk").
fn task<'a>(t: &'a Timeline, subject: &str) -> &'a timechopper::interpret::Task {
    t.tasks
        .iter()
        .find(|task| task.identity.as_ref().and_then(|a| a.subject.as_deref()) == Some(subject))
        .unwrap_or_else(|| panic!("no task about {subject}"))
}

/// The glance at mail during the billing work: a member swept into the
/// billing row, or a row of its own carved out of it.
fn glance(t: &Timeline) -> (bool, Option<timechopper::interpret::history::Precedent>) {
    let about_billing = |task: &timechopper::interpret::Task| {
        task.identity.as_ref().and_then(|a| a.subject.as_deref()) == Some("billing")
    };
    let billing = t.tasks.iter().position(about_billing).unwrap();
    for session in &t.sessions {
        for member in &session.members {
            let at = t.spans[member.spans[0]].start;
            if member.activity.subject.as_deref() == Some("Gmail") && at > Time::hms(9, 0, 0) {
                return (session.task == billing, member.precedent.clone());
            }
        }
    }
    panic!("no glance at mail");
}

/// Where the billing work starts, and the ticket, on any day of the week.
fn starts(day: &Day) -> (Time, Time) {
    let billing = day
        .spans
        .iter()
        .find(|s| s.window.title.contains("billing"))
        .unwrap();
    let ticket = day
        .spans
        .iter()
        .find(|s| s.window.title.contains("Ticket"))
        .unwrap();
    (billing.start, ticket.start)
}

#[test]
fn rules_are_offered_then_applied() {
    let days = week();
    // Monday: nothing to go on.
    let mon = view(&days, 0, &[]);
    assert!(
        mon.tasks
            .iter()
            .all(|t| t.precedent.is_none() && t.name.is_none())
    );
    assert_eq!(glance(&mon), (true, None));

    // Tuesday: Monday's names and separation are offered, not applied.
    let tue = view(&days, 1, &[]);
    let billing = task(&tue, "billing");
    assert_eq!(billing.name, None);
    let offer = billing.precedent.as_ref().unwrap();
    assert_eq!(
        (offer.name.as_deref(), offer.applied),
        (Some("Billing"), false)
    );
    assert_eq!(offer.days, ["Mon"]);
    let ticket = task(&tue, "Helpdesk").precedent.as_ref().unwrap();
    assert_eq!(ticket.name.as_deref(), Some("Ticket 123"));
    let (swept, offer) = glance(&tue);
    assert!(swept);
    assert!(!offer.unwrap().applied);

    // Wednesday: the ticket's name has changed from one day to the next, so
    // it says nothing; Billing is still only offered.
    let wed = view(&days, 2, &[]);
    assert!(task(&wed, "Helpdesk").precedent.is_none());
    let offer = task(&wed, "billing").precedent.as_ref().unwrap();
    assert_eq!(offer.days, ["Mon", "Tue"]);
    assert!(!offer.applied);

    // Thursday: three days in a row. Billing is named, the glance at mail is
    // a row of its own; nobody had to say so.
    let thu = view(&days, 3, &[]);
    assert_eq!(APPLY_AFTER, 3);
    let billing = task(&thu, "billing");
    assert_eq!(billing.name.as_deref(), Some("Billing"));
    assert!(billing.precedent.as_ref().unwrap().applied);
    assert!(task(&thu, "Helpdesk").name.is_none());
    let (swept, rule) = glance(&thu);
    assert!(!swept);
    assert!(rule.unwrap().applied);
    assert!(
        thu.sessions.iter().any(
            |s| s.parent.is_some() && s.members[0].activity.subject.as_deref() == Some("Gmail")
        )
    );
}

#[test]
fn todays_corrections_come_first() {
    let days = week();
    let (billing, _) = starts(&days[3]);
    let name = Hint::Name {
        at: billing,
        name: "Invoices".into(),
    };
    let thu = view(&days, 3, &[name]);
    assert_eq!(task(&thu, "billing").name.as_deref(), Some("Invoices"));
}

#[test]
fn going_against_a_rule_undoes_it() {
    let mut days = week();
    let (billing, _) = starts(&days[3]);

    // Thursday: the user takes the carried-over name away.
    days[3].hints = vec![Hint::Name {
        at: billing,
        name: String::new(),
    }];
    let thu = view(&days, 3, &days[3].hints.clone());
    assert_eq!(task(&thu, "billing").name, None);
    let fri = view(&days, 4, &[]);
    assert_eq!(task(&fri, "billing").name, None);
    assert!(task(&fri, "billing").precedent.is_none());

    // Renamed instead: the new name has to prove itself (a changed name is
    // offered only after two days of it).
    days[3].hints = vec![Hint::Name {
        at: billing,
        name: "Invoices".into(),
    }];
    let fri = view(&days, 4, &[]);
    assert_eq!(task(&fri, "billing").name, None);
    assert!(task(&fri, "billing").precedent.is_none());
    days[4].hints = days[3].hints.clone();
    days[4].hints[0] = Hint::Name {
        at: starts(&days[4]).0,
        name: "Invoices".into(),
    };
    days.push(Day {
        label: "Sat".into(),
        ..days[4].clone()
    });
    let sat = view(&days, 5, &[]);
    let offer = task(&sat, "billing").precedent.as_ref().unwrap();
    assert_eq!(
        (offer.name.as_deref(), offer.applied),
        (Some("Invoices"), false)
    );
    assert_eq!(offer.days, ["Thu", "Fri"]);
}

#[test]
fn joining_a_carved_out_glance_back_undoes_its_rule() {
    let mut days = week();
    let thu = view(&days, 3, &[]);
    let mail = thu
        .sessions
        .iter()
        .position(|s| {
            s.parent.is_some() && s.members[0].activity.subject.as_deref() == Some("Gmail")
        })
        .unwrap();
    let join = thu.join(mail, thu.sessions[mail].parent.unwrap()).unwrap();
    days[3].hints = vec![join];
    let thu = view(&days, 3, &days[3].hints.clone());
    assert!(glance(&thu).0);
    // Friday, the glance stays where it is, and isn't even offered.
    let fri = view(&days, 4, &[]);
    assert_eq!(glance(&fri), (true, None));
}

#[test]
fn everywhere_is_applied_from_the_next_day() {
    let mut days = week();
    for day in &mut days {
        day.hints.clear();
    }
    let gmail = timechopper::interpret::Pattern::parse("firefox:gmail").unwrap();
    days[0].hints = vec![Hint::apart(gmail)];
    let tue = view(&days, 1, &[]);
    let (swept, rule) = glance(&tue);
    assert!(!swept);
    assert_eq!(rule.unwrap().days, ["Mon"]);
}

#[test]
fn a_day_can_be_read_back_from_its_log() {
    let days = week();
    let mon = &days[0];
    let mut records = timechopper::live::records(&mon.spans, &mon.marks);
    let end = mon.spans.last().unwrap().end;
    for hint in &mon.hints {
        records.push(Record::Hint {
            at: end,
            hint: hint.clone(),
        });
    }
    records.push(Record::Hint {
        at: end,
        hint: Hint::Split { at: end },
    });
    records.push(Record::Unhint { at: end });
    let text: String = records
        .iter()
        .map(timechopper::interpret::model::record_line)
        .collect();
    let back = Day::from_records(
        "Mon",
        &timechopper::interpret::model::parse_records(&text).unwrap(),
    );
    assert!(back.spans == mon.spans);
    assert_eq!(back.hints, mon.hints);
}
