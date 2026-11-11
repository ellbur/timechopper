//! Synthetic focus logs: made-up working days, small and large, to try the
//! interpretation on without a real desktop.

use super::model::{Event, Span, Time, Window, spans};

/// Writes a focus log by saying how long each window was used.
pub struct Script {
    now: Time,
    events: Vec<Event>,
}

impl Script {
    pub fn at(h: u64, m: u64) -> Script {
        Script {
            now: Time::hms(h, m, 0),
            events: Vec::new(),
        }
    }

    fn push(&mut self, window: Option<Window>, secs: u64) -> &mut Script {
        self.events.push(Event {
            at: self.now,
            window,
        });
        self.now = Time(self.now.0 + secs);
        self
    }

    /// `app` has focus for `secs` seconds, with this title.
    pub fn focus(&mut self, app: &str, title: &str, secs: u64) -> &mut Script {
        let window = Window {
            app: app.to_string(),
            title: title.to_string(),
        };
        self.push(Some(window), secs)
    }

    /// Nothing is used for `secs` seconds.
    pub fn idle(&mut self, secs: u64) -> &mut Script {
        self.push(None, secs)
    }

    pub fn finish(&mut self) -> Vec<Span> {
        self.push(None, 0);
        spans(&self.events)
    }
}

pub const NOTEPAD: &str = "notepad.exe";
pub const FIREFOX: &str = "firefox.exe";
pub const OUTLOOK: &str = "OUTLOOK.EXE";
pub const CODE: &str = "Code.exe";
pub const TERMINAL: &str = "WindowsTerminal.exe";
pub const EXCEL: &str = "EXCEL.EXE";
pub const WORD: &str = "WINWORD.EXE";
pub const TEAMS: &str = "ms-teams.exe";

fn page(title: &str) -> String {
    format!("{title} — Mozilla Firefox")
}

fn code(file: &str, project: &str) -> String {
    format!("{file} - {project} - Visual Studio Code")
}

/// Writing a report in Notepad, looking things up in the browser.
fn report(s: &mut Script, email_in_the_middle: bool) {
    s.focus(NOTEPAD, "report.txt - Notepad", 180)
        .focus(FIREFOX, &page("How to structure a quarterly report"), 90)
        .focus(NOTEPAD, "*report.txt - Notepad", 240)
        .focus(FIREFOX, &page("Quarterly report examples"), 60)
        .focus(NOTEPAD, "*report.txt - Notepad", 300);
    if email_in_the_middle {
        s.focus(OUTLOOK, "Inbox - Outlook", 40);
    }
    s.focus(FIREFOX, &page("Thesaurus: synonyms for growth"), 45)
        .focus(NOTEPAD, "*report.txt - Notepad", 200)
        .focus(FIREFOX, &page("How to structure a quarterly report"), 75)
        .focus(NOTEPAD, "report.txt - Notepad", 240);
}

/// Back and forth between an editor and a browser: one task.
pub fn alternation() -> Vec<Span> {
    let mut s = Script::at(9, 0);
    report(&mut s, false);
    s.finish()
}

/// The same, with forty seconds of email in the middle.
pub fn carve_out() -> Vec<Span> {
    let mut s = Script::at(9, 0);
    report(&mut s, true);
    s.finish()
}

/// Notes in an unsaved Notepad window: only the browser says what they are
/// about.
pub fn uninformative() -> Vec<Span> {
    let mut s = Script::at(14, 0);
    s.focus(FIREFOX, &page("Kubernetes ingress timeout"), 200)
        .focus(NOTEPAD, "Untitled - Notepad", 120)
        .focus(
            FIREFOX,
            &page("nginx proxy_read_timeout - Stack Overflow"),
            90,
        )
        .focus(NOTEPAD, "*Untitled - Notepad", 60)
        .focus(FIREFOX, &page("Kubernetes ingress timeout"), 240)
        .focus(NOTEPAD, "*Untitled - Notepad", 150)
        .focus(FIREFOX, &page("Kubernetes ingress timeout"), 100)
        .focus(NOTEPAD, "*Untitled - Notepad", 80);
    s.finish()
}

/// A stretch of programming: mostly the editor, with trips to the docs, to
/// Stack Overflow and, every ten minutes or so, to a terminal whose title
/// never changes.
fn programming(s: &mut Script, project: &str, files: [&str; 2], docs: &str, rounds: usize) {
    for round in 0..rounds {
        s.focus(
            CODE,
            &code(files[round % 2], project),
            280 + 20 * (round as u64 % 3),
        );
        match round % 4 {
            1 | 3 => s.focus(TERMINAL, "PowerShell", 50),
            0 => s.focus(FIREFOX, &page(&format!("Page {round} - {docs}")), 70),
            _ => s.focus(
                FIREFOX,
                &page(&format!(
                    "Question {round} about {project} - Stack Overflow"
                )),
                60,
            ),
        };
    }
    s.focus(CODE, &code(files[0], project), 200);
}

/// One project, then another, in the same editor, browser and terminal.
pub fn project_switch() -> Vec<Span> {
    let mut s = Script::at(9, 0);
    programming(&mut s, "timechopper", ["main.rs", "win.rs"], "Rust", 7);
    programming(
        &mut s,
        "billing",
        ["invoice.py", "models.py"],
        "Django documentation",
        6,
    );
    s.finish()
}

/// A spreadsheet, a forty-minute meeting, the spreadsheet again.
pub fn resumed() -> Vec<Span> {
    let mut s = Script::at(13, 0);
    let budget = |s: &mut Script, rounds: u64| {
        for round in 0..rounds {
            s.focus(EXCEL, "budget-2027.xlsx - Excel", 240).focus(
                FIREFOX,
                &page(&format!("Exchange rates, table {round}")),
                50,
            );
        }
        s.focus(EXCEL, "budget-2027.xlsx - Excel", 180);
    };
    budget(&mut s, 5);
    s.focus(TEAMS, "Weekly sync | Microsoft Teams", 40 * 60);
    budget(&mut s, 4);
    s.finish()
}

/// A whole day: email now and then, two programming projects, a meeting,
/// a report written from a spreadsheet, and lunch.
pub fn messy_day() -> Vec<Span> {
    let mut s = Script::at(9, 0);
    s.focus(OUTLOOK, "Inbox - Outlook", 360);
    programming(&mut s, "timechopper", ["main.rs", "win.rs"], "Rust", 3);
    s.focus(OUTLOOK, "Inbox - Outlook", 30);
    programming(&mut s, "timechopper", ["win.rs", "main.rs"], "Rust", 4);
    s.focus(TEAMS, "Weekly sync | Microsoft Teams", 30 * 60);
    programming(&mut s, "timechopper", ["main.rs", "win.rs"], "Rust", 4);
    s.focus(OUTLOOK, "Inbox - Outlook", 45);
    programming(&mut s, "timechopper", ["win.rs", "main.rs"], "Rust", 2);
    for round in 0..4 {
        s.focus(WORD, "Q3 report.docx - Word", 260)
            .focus(EXCEL, "budget-2027.xlsx - Excel", 90);
        if round == 1 {
            s.focus(FIREFOX, &page("Exchange rates"), 40);
        }
    }
    s.focus(WORD, "Q3 report.docx - Word", 200);
    s.idle(45 * 60);
    s.focus(OUTLOOK, "Inbox - Outlook", 300);
    programming(
        &mut s,
        "billing",
        ["invoice.py", "models.py"],
        "Django documentation",
        9,
    );
    s.finish()
}

pub struct Scenario {
    pub name: &'static str,
    pub about: &'static str,
    pub spans: fn() -> Vec<Span>,
}

pub const ALL: [Scenario; 6] = [
    Scenario {
        name: "alternation",
        about: "Back and forth between Notepad and a browser.",
        spans: alternation,
    },
    Scenario {
        name: "carve-out",
        about: "The same, with forty seconds of email in the middle.",
        spans: carve_out,
    },
    Scenario {
        name: "uninformative",
        about: "An unsaved Notepad window; only the browser says what it is about.",
        spans: uninformative,
    },
    Scenario {
        name: "project-switch",
        about: "One project, then another, in the same editor, browser and terminal.",
        spans: project_switch,
    },
    Scenario {
        name: "resumed",
        about: "A spreadsheet, a forty-minute meeting, the spreadsheet again.",
        spans: resumed,
    },
    Scenario {
        name: "messy-day",
        about: "Email now and then, two projects, a meeting, a report, lunch.",
        spans: messy_day,
    },
];
