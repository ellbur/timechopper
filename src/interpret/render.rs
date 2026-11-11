//! A timeline as text.

use super::model::fmt_duration;
use super::{Label, Timeline};

fn describe(label: &Label) -> String {
    if label.apps.join(" + ") == label.headline {
        label.headline.clone()
    } else {
        format!("{}  [{}]", label.headline, label.apps.join(", "))
    }
}

/// One row per session, in time order, each showing the time actually spent
/// on it. A session carved out of another is indented under it; its time is
/// not part of the outer session's.
pub fn render(timeline: &Timeline) -> String {
    let mut out = String::new();
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); timeline.sessions.len()];
    let mut roots = Vec::new();
    for (i, session) in timeline.sessions.iter().enumerate() {
        match session.parent {
            Some(parent) => children[parent].push(i),
            None => roots.push(i),
        }
    }
    let mut stack: Vec<(usize, usize)> = roots.into_iter().rev().map(|i| (i, 0)).collect();
    while let Some((i, depth)) = stack.pop() {
        let session = &timeline.sessions[i];
        let task = &timeline.tasks[session.task];
        let indent = "    ".repeat(depth);
        let mut notes = Vec::new();
        if task.sessions.len() > 1 {
            let nth = task.sessions.iter().position(|&s| s == i).unwrap() + 1;
            notes.push(format!("part {nth} of {}", task.sessions.len()));
        }
        if session.visits > 1 {
            notes.push(format!("{} visits", session.visits));
        }
        if session.elsewhere() > 0 {
            notes.push(format!("{} elsewhere", fmt_duration(session.elsewhere())));
        }
        let notes = match notes.is_empty() {
            true => String::new(),
            false => format!("  ({})", notes.join(", ")),
        };
        out.push_str(&format!(
            "{indent}{}-{}  {:>6}  {}{notes}\n",
            session.start.hm(),
            session.end.hm(),
            fmt_duration(session.active),
            describe(&session.label),
        ));
        if !session.label.also.is_empty() {
            let also: Vec<String> = session
                .label
                .also
                .iter()
                .map(|e| format!("{} {}", e.text, fmt_duration(e.secs)))
                .collect();
            out.push_str(&format!("{indent}{:21}also: {}\n", "", also.join(" · ")));
        }
        stack.extend(children[i].iter().rev().map(|&c| (c, depth + 1)));
    }

    out.push_str("\nTotals\n");
    let mut order: Vec<usize> = (0..timeline.tasks.len()).collect();
    order.sort_by_key(|&t| std::cmp::Reverse(timeline.tasks[t].active));
    for t in order {
        let task = &timeline.tasks[t];
        out.push_str(&format!(
            "  {:>6}  {}\n",
            fmt_duration(task.active),
            describe(&task.label)
        ));
    }
    out.push_str(&format!(
        "  {:>6}  tracked\n",
        fmt_duration(timeline.tracked)
    ));
    out
}
