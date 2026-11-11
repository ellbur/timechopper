//! Reading window titles: which parts name the application, which parts say
//! what the user was looking at, and which of those are worth treating as an
//! identity.

use std::collections::{HashMap, HashSet};

use super::model::{Span, Window};

/// Titles are conventionally built from parts joined by one of these, with
/// the application's name last: `main.rs - timechopper - Visual Studio Code`.
const SEPARATORS: [&str; 5] = [" - ", " — ", " – ", " | ", " · "];

/// Title parts that say nothing about what the user was doing.
const UNINFORMATIVE: [&str; 2] = ["untitled", "new tab"];

/// How much of a window's title counts towards its identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lens {
    /// One part of the title: the part that, for this app, changes least
    /// from one visit to the next (a project rather than each of its files,
    /// a site rather than each of its pages), provided the window's value
    /// for it was returned to or outlasted a change of title. Windows that
    /// offer no such part are identified by their app alone.
    Auto,
    /// Ignore the title: all windows of the app are the same activity.
    App,
    /// The whole title, minus the app's name.
    Title,
}

/// What a window is taken to be, for the purpose of telling tasks apart.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Activity {
    pub app: String,
    pub subject: Option<String>,
}

impl std::fmt::Display for Activity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.subject {
            Some(subject) => write!(f, "{}[{subject}]", self.app),
            None => write!(f, "{}", self.app),
        }
    }
}

/// `C:\Windows\notepad.exe` -> `notepad`.
pub fn app_stem(app: &str) -> String {
    let name = app.rsplit(['\\', '/']).next().unwrap_or(app).to_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_string()
}

/// Splits a title into its parts, tidied so that the same thing reads the
/// same every time: unsaved-changes markers and unread counters are removed.
pub fn segments(title: &str) -> Vec<String> {
    let mut parts = vec![title];
    for sep in SEPARATORS {
        parts = parts.into_iter().flat_map(|p| p.split(sep)).collect();
    }
    parts
        .into_iter()
        .map(tidy)
        .filter(|p| !p.is_empty())
        .collect()
}

fn tidy(segment: &str) -> String {
    let mut out = String::new();
    let mut rest = segment;
    while let Some(open) = rest.find('(') {
        let after = &rest[open + 1..];
        match after.find(')') {
            Some(close) if close > 0 && after[..close].chars().all(|c| c.is_ascii_digit()) => {
                out.push_str(&rest[..open]);
                rest = &after[close + 1..];
            }
            _ => {
                out.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    let out = out
        .trim()
        .trim_start_matches(['*', '●'])
        .trim_end_matches('*');
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Does this title part name the application? True when one of its words is
/// the executable's name, or a long enough start or end of it (`Edge` for
/// `msedge`, `Word` for `winword`).
fn names_app(segment: &str, stem: &str) -> bool {
    segment
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .any(|w| w == stem || (w.len() >= 4 && (stem.starts_with(&w) || stem.ends_with(&w))))
}

/// The position of the part naming the app: the last part, else the first.
fn app_name_index(segments: &[String], stem: &str) -> Option<usize> {
    let last = segments.len().checked_sub(1)?;
    [last, 0]
        .into_iter()
        .find(|&i| names_app(&segments[i], stem))
}

pub(crate) fn is_informative(segment: &str) -> bool {
    !UNINFORMATIVE.contains(&segment.to_lowercase().as_str())
}

/// What the titles in one log say, learned from the log as a whole.
pub struct Titles {
    stems: Vec<String>,
    subjects: Vec<Vec<String>>,
    display: HashMap<String, String>,
    /// Every (app, title part) that was returned to, or that stayed while
    /// the rest of the title changed: an identity, not a passing page.
    recurring: HashSet<(String, String)>,
    /// Per app, the positions of title parts (counted from the app's name
    /// outwards, starting at 1) from the steadiest to the most changeable.
    steadiest: HashMap<String, Vec<usize>>,
}

impl Titles {
    pub fn learn(spans: &[Span]) -> Titles {
        let stems: Vec<String> = spans.iter().map(|s| app_stem(&s.window.app)).collect();
        let parts: Vec<Vec<String>> = spans.iter().map(|s| segments(&s.window.title)).collect();

        // Find each app's name as it appears in its titles.
        let mut named: HashMap<&str, HashMap<&str, usize>> = HashMap::new();
        let mut distinct: HashMap<&str, HashSet<&[String]>> = HashMap::new();
        for (stem, parts) in stems.iter().zip(&parts) {
            distinct.entry(stem).or_default().insert(parts);
            if let Some(i) = app_name_index(parts, stem) {
                *named.entry(stem).or_default().entry(&parts[i]).or_default() += 1;
            }
        }
        let mut display: HashMap<String, String> = HashMap::new();
        // Apps whose name is nothing like their executable's: if every title
        // ends the same way, that ending is the name.
        let mut constant_last: HashMap<&str, &str> = HashMap::new();
        for (stem, titles) in &distinct {
            if let Some(votes) = named.get(stem) {
                let best = votes.iter().max_by_key(|(name, n)| (**n, *name)).unwrap();
                display.insert(stem.to_string(), best.0.to_string());
                continue;
            }
            let mut lasts = titles.iter().map(|t| t.last().filter(|_| t.len() >= 2));
            if let Some(first) = lasts.next().flatten()
                && titles.len() >= 3
                && lasts.all(|l| l == Some(first))
            {
                constant_last.insert(stem, first);
                display.insert(stem.to_string(), first.to_string());
            }
        }

        let subjects: Vec<Vec<String>> = stems
            .iter()
            .zip(&parts)
            .map(|(stem, parts)| {
                let drop = app_name_index(parts, stem).or_else(|| {
                    let last = constant_last.get(stem.as_str())?;
                    (parts.last().map(String::as_str) == Some(*last)).then(|| parts.len() - 1)
                });
                let keep = |(i, p): (usize, &String)| (Some(i) != drop).then(|| p.clone());
                parts.iter().enumerate().filter_map(keep).collect()
            })
            .collect();

        // A title part is an identity, rather than a passing page, if it
        // was returned to or stayed while the rest of the title changed.
        #[derive(Default)]
        struct Seen<'a> {
            visits: usize,
            titles: HashSet<&'a [String]>,
        }
        let mut seen: HashMap<(String, String), Seen> = HashMap::new();
        for i in 0..spans.len() {
            for part in &subjects[i] {
                let continued = i > 0
                    && stems[i - 1] == stems[i]
                    && spans[i - 1].end == spans[i].start
                    && subjects[i - 1].contains(part);
                let entry = seen.entry((stems[i].clone(), part.clone())).or_default();
                entry.visits += usize::from(!continued);
                entry.titles.insert(&subjects[i]);
            }
        }
        let recurring = seen
            .into_iter()
            .filter(|((_, part), seen)| {
                (seen.visits >= 2 || seen.titles.len() >= 2) && is_informative(part)
            })
            .map(|(key, _)| key)
            .collect();

        // How often each position changes between one of the app's windows
        // and its next: (changes, comparisons).
        let mut changes: HashMap<&str, Vec<(u32, u32)>> = HashMap::new();
        let mut previous: HashMap<&str, &[String]> = HashMap::new();
        for (stem, subject) in stems.iter().zip(&subjects) {
            let counts = changes.entry(stem).or_default();
            if counts.len() < subject.len() {
                counts.resize(subject.len(), (0, 0));
            }
            // The same title twice says nothing about what changes.
            let before = previous.insert(stem, subject);
            if let Some(before) = before.filter(|before| *before != subject.as_slice()) {
                let pairs = before.iter().rev().zip(subject.iter().rev());
                for (count, (a, b)) in counts.iter_mut().zip(pairs) {
                    count.0 += u32::from(a != b);
                    count.1 += 1;
                }
            }
        }
        let steadiest = changes
            .into_iter()
            .map(|(stem, counts)| {
                let rate = |&(changed, compared): &(u32, u32)| match compared {
                    0 => 1.0,
                    _ => f64::from(changed) / f64::from(compared),
                };
                let mut order: Vec<usize> = (1..=counts.len()).collect();
                order.sort_by(|&a, &b| rate(&counts[a - 1]).total_cmp(&rate(&counts[b - 1])));
                (stem.to_string(), order)
            })
            .collect();

        Titles {
            stems,
            subjects,
            display,
            recurring,
            steadiest,
        }
    }

    /// The executable's name, lowercased, without `.exe`.
    pub fn stem(&self, span: usize) -> &str {
        &self.stems[span]
    }

    /// The span's title parts, minus the one naming the app.
    pub fn subject(&self, span: usize) -> &[String] {
        &self.subjects[span]
    }

    /// The app's name as its own titles spell it, else its executable's.
    pub fn display_name<'a>(&'a self, stem: &'a str) -> &'a str {
        self.display.get(stem).map_or(stem, String::as_str)
    }

    /// The identity of a span's window under a lens.
    pub fn activity(&self, span: usize, lens: Lens) -> Activity {
        let stem = &self.stems[span];
        let parts = &self.subjects[span];
        let subject = match lens {
            Lens::App => None,
            Lens::Title => Some(parts.join(" - ")).filter(|s| !s.is_empty()),
            Lens::Auto => self.steadiest[stem]
                .iter()
                .filter_map(|&position| parts.get(parts.len().checked_sub(position)?))
                .find(|part| self.recurring.contains(&(stem.clone(), (*part).clone())))
                .cloned(),
        };
        Activity {
            app: stem.clone(),
            subject,
        }
    }
}

/// Selects windows by app and/or by text in the title, ignoring case.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pattern {
    pub app: Option<String>,
    pub title: Option<String>,
}

impl Pattern {
    pub fn app(app: &str) -> Pattern {
        Pattern {
            app: Some(app_stem(app)),
            title: None,
        }
    }

    pub fn title(text: &str) -> Pattern {
        Pattern {
            app: None,
            title: Some(text.to_lowercase()),
        }
    }

    /// `APP`, `APP:TEXT` or `:TEXT`.
    pub fn parse(text: &str) -> Result<Pattern, String> {
        let (app, title) = text.split_once(':').unwrap_or((text, ""));
        if app.is_empty() && title.is_empty() {
            return Err("empty pattern".to_string());
        }
        Ok(Pattern {
            app: (!app.is_empty()).then(|| app_stem(app)),
            title: (!title.is_empty()).then(|| title.to_lowercase()),
        })
    }

    pub fn matches(&self, window: &Window) -> bool {
        self.app
            .as_ref()
            .is_none_or(|a| *a == app_stem(&window.app))
            && self
                .title
                .as_ref()
                .is_none_or(|t| window.title.to_lowercase().contains(t))
    }
}

impl std::fmt::Display for Pattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.app, &self.title) {
            (Some(app), Some(title)) => write!(f, "{app}:{title}"),
            (Some(app), None) => write!(f, "{app}"),
            (None, title) => write!(f, ":{}", title.as_deref().unwrap_or("")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::Time;
    use super::*;

    fn log(windows: &[(&str, &str)]) -> Vec<Span> {
        windows
            .iter()
            .enumerate()
            .map(|(i, (app, title))| Span {
                start: Time(i as u64 * 60),
                end: Time(i as u64 * 60 + 60),
                window: Window {
                    app: app.to_string(),
                    title: title.to_string(),
                },
            })
            .collect()
    }

    #[test]
    fn splits_and_tidies() {
        assert_eq!(
            segments("*main.rs - timechopper - Visual Studio Code"),
            ["main.rs", "timechopper", "Visual Studio Code"]
        );
        assert_eq!(
            segments("Inbox (3) | Gmail — Mozilla Firefox"),
            ["Inbox", "Gmail", "Mozilla Firefox"]
        );
        assert_eq!(
            segments("f(x) (draft) - Notepad"),
            ["f(x) (draft)", "Notepad"]
        );
        assert_eq!(app_stem(r"C:\Windows\System32\Notepad.EXE"), "notepad");
    }

    #[test]
    fn finds_app_names() {
        let spans = log(&[
            ("notepad.exe", "note - Notepad"),
            ("msedge.exe", "Some page - Personal - Microsoft Edge"),
            ("Code.exe", "decode.rs - Visual Studio Code"),
            ("CalculatorApp.exe", "Calculator"),
            ("devenv.exe", "a.cs - Microsoft Visual Studio"),
            ("devenv.exe", "b.cs - Microsoft Visual Studio"),
            ("devenv.exe", "c.cs - Microsoft Visual Studio"),
        ]);
        let titles = Titles::learn(&spans);
        assert_eq!(titles.subject(0), ["note"]);
        assert_eq!(titles.subject(1), ["Some page", "Personal"]);
        assert_eq!(titles.subject(2), ["decode.rs"]);
        assert!(titles.subject(3).is_empty());
        assert_eq!(titles.subject(4), ["a.cs"]);
        assert_eq!(titles.display_name("msedge"), "Microsoft Edge");
        assert_eq!(titles.display_name("devenv"), "Microsoft Visual Studio");
        assert_eq!(titles.display_name("unknown"), "unknown");
    }

    #[test]
    fn auto_lens_keeps_what_recurs() {
        let spans = log(&[
            ("Code.exe", "main.rs - timechopper - Visual Studio Code"),
            ("firefox.exe", "A page read once — Mozilla Firefox"),
            ("Code.exe", "win.rs - timechopper - Visual Studio Code"),
            ("notepad.exe", "Untitled - Notepad"),
            ("Code.exe", "main.rs - timechopper - Visual Studio Code"),
            ("notepad.exe", "Untitled - Notepad"),
        ]);
        let titles = Titles::learn(&spans);
        let auto = |i| titles.activity(i, Lens::Auto).to_string();
        // The project stays put while the file changes.
        assert_eq!(auto(0), "code[timechopper]");
        assert_eq!(auto(2), "code[timechopper]");
        // Seen once: not an identity.
        assert_eq!(auto(1), "firefox");
        // Recurs, but says nothing.
        assert_eq!(auto(3), "notepad");
        assert_eq!(
            titles.activity(0, Lens::Title).to_string(),
            "code[main.rs - timechopper]"
        );
        assert_eq!(titles.activity(0, Lens::App).to_string(), "code");
    }

    #[test]
    fn auto_lens_goes_by_position_not_by_value() {
        // Both projects have a main.rs, on which more time is spent than on
        // either project; it is still the project that identifies a window.
        let spans = log(&[
            ("Code.exe", "main.rs - alpha - Visual Studio Code"),
            ("Code.exe", "util.rs - alpha - Visual Studio Code"),
            ("Code.exe", "main.rs - alpha - Visual Studio Code"),
            ("Code.exe", "main.rs - beta - Visual Studio Code"),
            ("Code.exe", "util.rs - beta - Visual Studio Code"),
            ("Code.exe", "main.rs - beta - Visual Studio Code"),
        ]);
        let titles = Titles::learn(&spans);
        let auto = |i| titles.activity(i, Lens::Auto).to_string();
        assert_eq!(auto(0), "code[alpha]");
        assert_eq!(auto(1), "code[alpha]");
        assert_eq!(auto(3), "code[beta]");
    }

    #[test]
    fn patterns() {
        let w = Window {
            app: "Firefox.exe".to_string(),
            title: "Inbox - Gmail — Mozilla Firefox".to_string(),
        };
        assert!(Pattern::parse("firefox").unwrap().matches(&w));
        assert!(Pattern::parse("firefox.exe:gmail").unwrap().matches(&w));
        assert!(Pattern::parse(":GMAIL").unwrap().matches(&w));
        assert!(!Pattern::parse("chrome:gmail").unwrap().matches(&w));
        assert!(!Pattern::parse("firefox:jira").unwrap().matches(&w));
        assert!(Pattern::parse(":").is_err());
    }
}
