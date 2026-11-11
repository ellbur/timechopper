//! A timeline as text.

use super::model::fmt_duration;
use super::{Doubt, Label, Member, Timeline, Why};

/// How much to say about what each row is made of.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Detail {
    /// Only what is in doubt.
    #[default]
    Doubts,
    /// Everything that was swept into each row.
    Contents,
}

fn describe(name: Option<&String>, label: &Label) -> String {
    let apps = label.apps.join(", ");
    let about = match label.apps.join(" + ") == label.headline {
        true => label.headline.clone(),
        false => format!("{}  [{apps}]", label.headline),
    };
    match name {
        Some(name) => format!("\"{name}\": {about}"),
        None => about,
    }
}

fn doubts(member: &Member) -> Vec<String> {
    let mut out = Vec::new();
    for doubt in &member.doubts {
        // If it also got in on a weak reason, say so.
        if member.why == Why::SameApp {
            out.push("seen once, in an app the task uses".to_string());
        }
        out.push(match doubt {
            Doubt::Elsewhere { own: true, .. } => "also a task of its own".to_string(),
            Doubt::Elsewhere { tasks: 1, .. } => "also in another task".to_string(),
            Doubt::Elsewhere { tasks, .. } => format!("also in {tasks} other tasks"),
        });
    }
    out
}

/// One row per session, in time order, each showing the time actually spent
/// on it. A session carved out of another is indented under it; its time is
/// not part of the outer session's. Lines starting with `?` point at what
/// the guess is least sure of; lines starting with `>` are what the user
/// marked along the way.
pub fn render(timeline: &Timeline, detail: Detail) -> String {
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
        if session.confirmed {
            notes.push("confirmed".to_string());
        }
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
            describe(task.name.as_ref(), &session.label),
        ));
        // Detail lines sit under the row's description, with their marker
        // in the two columns before it.
        let mut line = |marker: char, text: String| {
            out.push_str(&format!("{indent}{:19}{marker} {text}\n", ""));
        };
        // The user's own markers: what they said at the time.
        if let Some(k) = session.opened_by {
            let m = &timeline.marks[k];
            line('>', format!("{}  {} (starts here)", m.at.hm(), m.mark));
        }
        for &k in &session.marks {
            let m = &timeline.marks[k];
            line('>', format!("{}  {}", m.at.hm(), m.mark));
        }
        if detail == Detail::Doubts && !session.label.also.is_empty() {
            let also: Vec<String> = session
                .label
                .also
                .iter()
                .map(|e| format!("{} {}", e.text, fmt_duration(e.secs)))
                .collect();
            line(' ', format!("also: {}", also.join(" · ")));
        }
        let width = session.members.iter().map(|m| m.name.chars().count());
        let width = width.max().unwrap_or(0);
        for member in &session.members {
            let doubts = doubts(member);
            if doubts.is_empty() && (detail == Detail::Doubts || session.members.len() == 1) {
                continue;
            }
            let visits = match member.visits {
                1 => "1 visit".to_string(),
                n => format!("{n} visits"),
            };
            let mut text = format!(
                "{:width$}  {:>6}  {visits}",
                member.name,
                fmt_duration(member.secs)
            );
            if !doubts.is_empty() {
                text.push_str(&format!("  - {}", doubts.join("; ")));
            }
            line(if doubts.is_empty() { '·' } else { '?' }, text);
        }
        for split in &session.splits {
            let text = format!(
                "{}  {} may give way to {} here",
                split.at.hm(),
                split.before,
                split.after
            );
            line('?', text);
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
            describe(task.name.as_ref(), &task.label)
        ));
    }
    out.push_str(&format!(
        "  {:>6}  tracked\n",
        fmt_duration(timeline.tracked)
    ));
    out
}
