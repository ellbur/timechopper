//! Finding tasks in the order in which activities follow each other.
//!
//! The input is a sequence of visits, each to one activity. The output is a
//! partition of those visits into tasks, found by three rules applied over
//! and over to smaller and smaller stretches:
//!
//! 1. **Clean cut.** If nothing that happens before a point also happens
//!    after it, the two sides are different tasks.
//! 2. **Island.** Inside a stretch that can't be cut, a run of visits to
//!    apps used nowhere else in the stretch is an interruption: a task of
//!    its own, sitting inside the task around it. What it interrupts carries
//!    on as one task. This rule goes by app, not by activity: a page seen
//!    once in a browser the task keeps returning to is part of the task.
//! 3. **Drift.** A stretch with no clean cut may still change character: the
//!    editor moves from one project to another while the same browser stays
//!    in use throughout. It is cut where the activities found on one side
//!    only make up most of the time on both sides.
//!
//! Whatever is left when no rule applies is one task: activities the user
//! kept going back and forth between.

use std::collections::HashMap;

/// One visit to an activity.
#[derive(Clone, Debug)]
pub(crate) struct Visit {
    /// The activity, or rather one bout of it: an activity dropped for a
    /// long time and picked up again counts as a new one for rules 1 and 3,
    /// so that something used all day doesn't tie the whole day together.
    pub sym: usize,
    pub secs: u64,
    pub app: usize,
}

/// Partitions visits into tasks. Each task lists the indexes of its visits
/// in order; tasks come out ordered by their first visit.
pub(crate) fn segment(visits: &[Visit], min_exclusive_share: f64) -> Vec<Vec<usize>> {
    let mut tasks = Vec::new();
    split(visits, 0, visits.len(), min_exclusive_share, &mut tasks);
    tasks.sort();
    tasks
}

fn split(visits: &[Visit], lo: usize, hi: usize, min_share: f64, tasks: &mut Vec<Vec<usize>>) {
    if lo >= hi {
        return;
    }
    let mut last: HashMap<usize, usize> = HashMap::new();
    for (i, visit) in visits.iter().enumerate().take(hi).skip(lo) {
        last.insert(visit.sym, i);
    }

    // Rule 1: cut wherever nothing reaches across.
    let mut blocks = Vec::new();
    let (mut start, mut reach) = (lo, lo);
    for (i, visit) in visits.iter().enumerate().take(hi).skip(lo) {
        reach = reach.max(last[&visit.sym]);
        if reach == i {
            blocks.push((start, i + 1));
            start = i + 1;
        }
    }
    if blocks.len() > 1 {
        for (start, end) in blocks {
            split(visits, start, end, min_share, tasks);
        }
        return;
    }

    // Rule 2: the largest stretches strictly inside the block whose apps
    // appear nowhere else in it.
    let mut app_first: HashMap<usize, usize> = HashMap::new();
    let mut app_last: HashMap<usize, usize> = HashMap::new();
    for (i, visit) in visits.iter().enumerate().take(hi).skip(lo) {
        app_first.entry(visit.app).or_insert(i);
        app_last.insert(visit.app, i);
    }
    let mut islands = Vec::new();
    let mut inside = vec![false; hi - lo];
    let mut i = lo + 1;
    while i < hi {
        let (mut earliest, mut latest, mut end) = (usize::MAX, 0, None);
        for (j, visit) in visits.iter().enumerate().take(hi - 1).skip(i) {
            earliest = earliest.min(app_first[&visit.app]);
            latest = latest.max(app_last[&visit.app]);
            if earliest < i {
                break;
            }
            if latest == j {
                end = Some(j + 1);
            }
        }
        match end {
            Some(end) => {
                islands.push((i, end));
                inside[i - lo..end - lo].fill(true);
                i = end;
            }
            None => i += 1,
        }
    }
    let frame: Vec<usize> = (lo..hi).filter(|i| !inside[i - lo]).collect();

    // Rule 3: cut where each side is mostly made of what the other lacks.
    let mut total: HashMap<usize, u64> = HashMap::new();
    let mut first_pos: HashMap<usize, usize> = HashMap::new();
    let mut last_pos: HashMap<usize, usize> = HashMap::new();
    for (pos, &i) in frame.iter().enumerate() {
        *total.entry(visits[i].sym).or_default() += visits[i].secs;
        first_pos.entry(visits[i].sym).or_insert(pos);
        last_pos.insert(visits[i].sym, pos);
    }
    let all: u64 = total.values().sum();
    let (mut left, mut left_only, mut right_only) = (0, 0, all);
    let mut best: Option<(f64, usize)> = None;
    for (pos, &i) in frame.iter().enumerate().take(frame.len() - 1) {
        let sym = visits[i].sym;
        left += visits[i].secs;
        if last_pos[&sym] == pos {
            left_only += total[&sym];
        }
        if first_pos[&sym] == pos {
            right_only -= total[&sym];
        }
        if left == 0 || left == all {
            continue;
        }
        let share = |part: u64, whole: u64| part as f64 / whole as f64;
        let score = share(left_only, left).min(share(right_only, all - left));
        if best.is_none_or(|(s, _)| score >= s) {
            best = Some((score, frame[pos + 1]));
        }
    }
    if let Some((score, cut)) = best
        && score > 0.0
        && score >= min_share
    {
        split(visits, lo, cut, min_share, tasks);
        split(visits, cut, hi, min_share, tasks);
        return;
    }

    tasks.push(frame);
    for (start, end) in islands {
        split(visits, start, end, min_share, tasks);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each letter is a visit of one minute; uppercase letters are visits of
    /// five. Lowercase and uppercase are the same activity.
    fn tasks(sequence: &str) -> Vec<String> {
        let visits: Vec<Visit> = sequence
            .chars()
            .map(|c| Visit {
                sym: c.to_ascii_lowercase() as usize,
                secs: if c.is_uppercase() { 300 } else { 60 },
                app: c.to_ascii_lowercase() as usize,
            })
            .collect();
        segment(&visits, 0.5)
            .into_iter()
            .map(|task| {
                let chars: Vec<char> = sequence.chars().collect();
                task.into_iter().map(|i| chars[i]).collect()
            })
            .collect()
    }

    #[test]
    fn alternation_is_one_task() {
        assert_eq!(tasks("ababab"), ["ababab"]);
    }

    #[test]
    fn clean_cuts_separate_tasks() {
        assert_eq!(tasks("ababcdcd"), ["abab", "cdcd"]);
        assert_eq!(tasks("abc"), ["a", "b", "c"]);
    }

    #[test]
    fn interruption_is_carved_out() {
        assert_eq!(tasks("ababmabab"), ["abababab", "m"]);
        // Two unrelated interruptions in a row.
        assert_eq!(tasks("abamnbab"), ["ababab", "m", "n"]);
        // An interruption with structure of its own, itself interrupted.
        assert_eq!(tasks("abamnmxnmbab"), ["ababab", "mnmnm", "x"]);
    }

    #[test]
    fn something_returned_to_belongs() {
        assert_eq!(tasks("abmabmab"), ["abmabmab"]);
    }

    #[test]
    fn drift_cuts_when_the_main_activity_changes() {
        // b stays in use throughout; the long visits move from a to c.
        assert_eq!(tasks("AbAbACbCbC"), ["AbAbA", "CbCbC"]);
        // The main activity a stays; only the helper changes.
        assert_eq!(tasks("AbAbAcAcA"), ["AbAbAcAcA"]);
        // The shared activity dominates: no grounds to cut.
        assert_eq!(tasks("aBaBaBcBcBc"), ["aBaBaBcBcBc"]);
    }

    #[test]
    fn interruptions_go_by_app() {
        let visit = |sym, app| Visit { sym, secs: 60, app };
        // Activity 0 is an editor. 1 and 2 are different sites in the same
        // browser, each seen once; 3 is another app, seen once.
        let visits = [
            visit(0, 0),
            visit(1, 1),
            visit(0, 0),
            visit(3, 2),
            visit(0, 0),
            visit(2, 1),
            visit(0, 0),
        ];
        assert_eq!(segment(&visits, 0.5), [vec![0, 1, 2, 4, 5, 6], vec![3]]);
    }
}
