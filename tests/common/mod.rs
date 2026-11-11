//! What must hold for any log under any hints.

use timechopper::interpret::{Span, Timeline};

/// Every second of every span is counted in exactly one member of exactly
/// one session of exactly one task.
pub fn check(log: &[Span], timeline: &Timeline) {
    let tracked: u64 = log.iter().map(Span::secs).sum();
    assert_eq!(timeline.tracked, tracked);
    assert_eq!(
        timeline.tasks.iter().map(|t| t.active).sum::<u64>(),
        tracked
    );
    assert_eq!(
        timeline.sessions.iter().map(|s| s.active).sum::<u64>(),
        tracked
    );
    // Hints may cut spans in two, but never move or lose time.
    let spans = &timeline.spans;
    assert_eq!(spans.iter().map(Span::secs).sum::<u64>(), tracked);
    assert!(spans.windows(2).all(|pair| pair[0].end <= pair[1].start));

    let mut owner = vec![None; spans.len()];
    for (i, session) in timeline.sessions.iter().enumerate() {
        assert!(timeline.tasks[session.task].sessions.contains(&i));
        assert!(session.active <= session.end.0 - session.start.0);
        let mut active = 0;
        for &span in &session.spans {
            assert_eq!(owner[span].replace(i), None, "span in two sessions");
            assert!(session.start <= spans[span].start && spans[span].end <= session.end);
            active += spans[span].secs();
        }
        assert_eq!(session.active, active);
        if let Some(parent) = session.parent {
            let parent = &timeline.sessions[parent];
            assert!(parent.start <= session.start && session.end <= parent.end);
            assert!(parent.elsewhere() >= session.active);
        }

        // Its members account for all of it, most time first.
        let mut in_members: Vec<usize> = Vec::new();
        for member in &session.members {
            let secs: u64 = member.spans.iter().map(|&i| spans[i].secs()).sum();
            assert_eq!(member.secs, secs);
            assert!(member.visits >= 1);
            in_members.extend(&member.spans);
        }
        in_members.sort();
        assert_eq!(in_members, session.spans);
        assert!(session.members.is_sorted_by(|a, b| a.secs >= b.secs));
        for split in &session.splits {
            assert!(session.start < split.at && split.at < session.end);
        }
    }
    assert!(owner.iter().all(Option::is_some), "span in no session");

    for (t, task) in timeline.tasks.iter().enumerate() {
        let sessions = task.sessions.iter().map(|&s| &timeline.sessions[s]);
        assert_eq!(sessions.clone().map(|s| s.active).sum::<u64>(), task.active);
        assert!(sessions.clone().all(|s| s.task == t));
        // A task's own sessions follow one another, unless the user fixed
        // one of them in place.
        let guessed = sessions.filter(|s| !s.confirmed);
        assert!(
            guessed
                .clone()
                .zip(guessed.skip(1))
                .all(|(a, b)| a.end < b.start)
        );
    }
}
