//! timechopper: watch the focused (foreground) window and print what can be
//! learned about it; interpret a log of focus changes as a timeline of tasks.

#[cfg(windows)]
mod win;

use std::process::ExitCode;

use timechopper::interpret::render::{Detail, render};
use timechopper::interpret::{
    Hint, Journal, Lens, Marker, Params, Pattern, Record, Span, Time, model, scenarios,
};

const USAGE: &str = "\
Usage: timechopper [--once] [--exit-after <SECONDS>]
       timechopper summarize <LOG> [INTERPRETATION OPTIONS]
       timechopper demo [<SCENARIO>] [--log] [INTERPRETATION OPTIONS]
       timechopper ui [--log-file <FILE>] [--idle <MINUTES>] [--max-gap <MINUTES>]
       timechopper ui --replay <SCENARIO> [--speed <N>] [--max-gap <MINUTES>]

With no command, watches the foreground window (Windows only) and prints
information about it every time focus moves to another window or the focused
window's title changes.

  --once                  Print the current foreground window and exit
  --exit-after <SECONDS>  Stop watching after this many seconds
  -h, --help              Show this help

summarize reads a focus log (one line per focus change: HH:MM:SS APP TITLE,
with APP '-' when nothing is in use) and prints the tasks it seems to show.
demo does the same for built-in synthetic logs; --log prints the log itself.
`demo week` shows a synthetic week, each day as it first looks with what the
days before it teach: names and separations offered after one day, applied
after three.

ui opens a small window in the corner of the screen showing the day so far,
grouped into tasks, up to the window in focus now, and lets you correct the
grouping as you go. It watches the foreground window (Windows only) and
keeps the focus log in <FILE> (default: a file per day under
%LOCALAPPDATA%\\timechopper), carrying on from it if it exists. Time with no
keyboard or mouse input for <MINUTES> (default 5) counts as away.
With --replay it plays a built-in synthetic log instead, <N> times faster
than real time (default 60), on any platform.

Each row is followed by '?' lines pointing at what the guess is least sure of,
and '>' lines with what was marked during it (in a log: HH:MM:SS @note TEXT,
@break TEXT, @opened APP TITLE, @closed APP TITLE). A marker settles a split
the guess would only offer; it never starts a task by itself.

Interpretation options:
  --contents                      List everything swept into each row
  --history                       Learn from the earlier logs in the same
                                  folder (summarize; ui always does)
  --until <TIME>                  Interpret the log as it stood at TIME
  --max-gap <MINUTES>             Longest interruption a task carries on
                                  through (default 15)
To correct the guess:
  --apart <PATTERN>[@<TIME>-<TIME>]
                                  These windows are a task of their own
                                  (always, or between those times)
  --together <PATTERN> <PATTERN>  These windows belong to the same task
  --lens <APP>=<auto|app|title>   How much of this app's titles tells its
                                  windows apart
  --split <TIME>                  The task in focus at TIME is a different
                                  task from then on
  --join <TIME> <TIME>            The row in focus at the first TIME belongs
                                  to the task in focus at the second
  --name <TIME> <NAME>            What to call the task in focus at TIME
A PATTERN is APP, APP:TEXT or :TEXT, where TEXT is part of a window title.
A TIME is HH:MM or HH:MM:SS.
";

pub struct Options {
    pub once: bool,
    pub exit_after: Option<std::time::Duration>,
}

enum Command {
    Watch(Options),
    Summarize {
        log: String,
        reading: Reading,
    },
    Demo {
        scenario: Option<String>,
        log: bool,
        reading: Reading,
    },
    Ui(UiOptions),
}

pub struct UiOptions {
    pub replay: Option<String>,
    pub speed: f64,
    pub log_file: Option<std::path::PathBuf>,
    pub idle: std::time::Duration,
    pub params: Params,
}

#[derive(Clone, Default)]
struct Reading {
    hints: Vec<Hint>,
    params: Params,
    detail: Detail,
    until: Option<Time>,
    /// Learn from the earlier logs in the same folder.
    history: bool,
    lessons: timechopper::interpret::history::Lessons,
}

fn time(text: &str) -> Result<Time, String> {
    Time::parse(text).ok_or(format!("invalid time: {text}"))
}

impl Reading {
    /// Consumes one interpretation option, if `arg` is one.
    fn parse(
        &mut self,
        arg: &str,
        args: &mut impl Iterator<Item = String>,
    ) -> Result<bool, String> {
        let mut value = |what: &str| args.next().ok_or(format!("{arg} needs {what}"));
        match arg {
            "--contents" => self.detail = Detail::Contents,
            "--history" => self.history = true,
            "--until" => self.until = Some(time(&value("a time")?)?),
            "--apart" => {
                let v = value("a pattern")?;
                // A trailing @FROM-TO limits the pattern to those times.
                let range = v.rsplit_once('@').and_then(|(pattern, range)| {
                    let (from, to) = range.split_once('-')?;
                    Some((pattern, (Time::parse(from)?, Time::parse(to)?)))
                });
                let (pattern, during) = match range {
                    Some((pattern, range)) => (pattern, vec![range]),
                    None => (v.as_str(), Vec::new()),
                };
                let pattern = Pattern::parse(pattern)?;
                self.hints.push(Hint::Apart { pattern, during });
            }
            "--together" => {
                let a = Pattern::parse(&value("two patterns")?)?;
                let b = Pattern::parse(&value("two patterns")?)?;
                self.hints.push(Hint::Together(a, b));
            }
            "--lens" => {
                let v = value("APP=LENS")?;
                let (app, lens) = v.split_once('=').ok_or(format!("expected APP=LENS: {v}"))?;
                let lens = match lens {
                    "auto" => Lens::Auto,
                    "app" => Lens::App,
                    "title" => Lens::Title,
                    _ => return Err(format!("unknown lens: {lens}")),
                };
                self.hints.push(Hint::Lens {
                    app: app.to_string(),
                    lens,
                });
            }
            "--split" => {
                let at = time(&value("a time")?)?;
                self.hints.push(Hint::Split { at });
            }
            "--join" => {
                let from = time(&value("two times")?)?;
                let into = time(&value("two times")?)?;
                self.hints.push(Hint::Join { from, into });
            }
            "--name" => {
                let at = time(&value("a time and a name")?)?;
                let name = value("a time and a name")?;
                self.hints.push(Hint::Name { at, name });
            }
            "--max-gap" => {
                let v = value("a number of minutes")?;
                let minutes: u64 = v.parse().map_err(|_| format!("invalid minutes: {v}"))?;
                self.params.max_gap = minutes * 60;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}

fn parse_args() -> Result<Command, String> {
    let mut args = std::env::args().skip(1).peekable();
    let command = args.next_if(|a| a == "summarize" || a == "demo" || a == "ui");
    let mut opts = Options {
        once: false,
        exit_after: None,
    };
    let mut reading = Reading::default();
    let mut positional = None;
    let mut log = false;
    let mut ui = UiOptions {
        replay: None,
        speed: 60.0,
        log_file: None,
        idle: std::time::Duration::from_secs(5 * 60),
        params: Params::default(),
    };
    let number = |arg: &str, v: Option<String>| -> Result<f64, String> {
        let v = v.ok_or(format!("{arg} needs a number"))?;
        v.parse()
            .ok()
            .filter(|n: &f64| n.is_finite() && *n > 0.0)
            .ok_or(format!("invalid number: {v}"))
    };
    while let Some(arg) = args.next() {
        match (arg.as_str(), command.as_deref()) {
            ("-h" | "--help", _) => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            ("--once", None) => opts.once = true,
            ("--exit-after", None) => {
                let v = args.next().ok_or("--exit-after needs a value")?;
                let secs: f64 = v
                    .parse()
                    .ok()
                    .filter(|s: &f64| s.is_finite() && *s >= 0.0)
                    .ok_or_else(|| format!("invalid number of seconds: {v}"))?;
                opts.exit_after = Some(std::time::Duration::from_secs_f64(secs));
            }
            ("--log", Some("demo")) => log = true,
            ("--replay", Some("ui")) => {
                ui.replay = Some(args.next().ok_or("--replay needs a scenario")?)
            }
            ("--speed", Some("ui")) => ui.speed = number(&arg, args.next())?,
            ("--log-file", Some("ui")) => {
                ui.log_file = Some(args.next().ok_or("--log-file needs a file")?.into())
            }
            ("--idle", Some("ui")) => {
                ui.idle = std::time::Duration::from_secs_f64(number(&arg, args.next())? * 60.0)
            }
            ("--max-gap", Some("ui")) => {
                ui.params.max_gap = (number(&arg, args.next())? * 60.0) as u64
            }
            (a, Some(_)) if reading.parse(a, &mut args)? => {}
            (a, Some(_)) if !a.starts_with('-') && positional.is_none() => positional = Some(arg),
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }
    Ok(match command.as_deref() {
        Some("summarize") => Command::Summarize {
            log: positional.ok_or("summarize needs a log file")?,
            reading,
        },
        Some("ui") => Command::Ui(ui),
        Some(_) => Command::Demo {
            scenario: positional,
            log,
            reading,
        },
        None => Command::Watch(opts),
    })
}

fn main() -> ExitCode {
    let result = match parse_args() {
        Ok(Command::Watch(opts)) => return run(opts),
        Ok(Command::Ui(opts)) => live(opts),
        Ok(Command::Summarize { log, reading }) => summarize(&log, &reading),
        Ok(Command::Demo {
            scenario,
            log,
            reading,
        }) => demo(scenario.as_deref(), log, &reading),
        Err(e) => {
            eprintln!("timechopper: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("timechopper: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Prints the interpretation of a log, with the corrections kept in it
/// (`logged`) followed by those given on the command line.
fn print_timeline(spans: &[Span], marks: &[Marker], logged: &[Hint], reading: &Reading) {
    // Replay the log the way the watcher would feed it, stopping early if
    // asked to show how things stood at some point.
    let mut journal = Journal::new(reading.params.clone());
    journal.learn(reading.lessons.clone());
    let end = spans.last().map_or(Time(0), |s| s.end);
    let now = reading.until.map_or(end, |t| t.min(end));
    let mut free = Time(0);
    for span in spans.iter().filter(|s| s.start < now) {
        if span.start > free {
            journal.observe(free, None);
        }
        journal.observe(span.start, Some(span.window.clone()));
        free = span.end;
    }
    if free < now {
        journal.observe(free, None);
    }
    for marker in marks {
        journal.mark(marker.clone());
    }
    for hint in logged.iter().chain(&reading.hints) {
        journal.apply(hint.clone());
    }
    let timeline = journal.timeline(now);
    println!(
        "{} focus changes, {} rows:\n",
        journal.spans(now).len(),
        timeline.sessions.len()
    );
    print!("{}", render(&timeline, reading.detail));
}

/// What the logs before the one at `path` teach (see
/// `timechopper::interpret::history`).
fn learn(path: &std::path::Path, params: &Params) -> timechopper::interpret::history::Lessons {
    let (days, skipped) = timechopper::live::earlier_days(path, 20);
    for file in skipped {
        eprintln!("timechopper: skipping an earlier log: {file}");
    }
    timechopper::interpret::history::Lessons::learn(&days, params)
}

fn summarize(path: &str, reading: &Reading) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let records = model::parse_records(&text).map_err(|e| format!("{path}: {e}"))?;
    let events: Vec<_> = records
        .iter()
        .filter_map(|r| match r {
            Record::Focus(e) => Some(e.clone()),
            _ => None,
        })
        .collect();
    // The corrections still standing at the end of the log.
    let mut hints = Journal::default();
    for record in &records {
        if let Record::Hint { .. } | Record::Unhint { .. } = record {
            hints.record(record.clone());
        }
    }
    let marks = model::markers(&records);
    let mut reading = reading.clone();
    if reading.history {
        reading.lessons = learn(std::path::Path::new(path), &reading.params);
    }
    print_timeline(&model::spans(&events), &marks, hints.hints(), &reading);
    Ok(())
}

/// The synthetic week: each day as it would look with what the days before
/// it teach.
fn week(reading: &Reading) -> Result<(), String> {
    use timechopper::interpret::history::Lessons;
    let days = scenarios::week();
    for (n, day) in days.iter().enumerate() {
        let lessons = Lessons::learn(&days[..n], &reading.params);
        let mut journal = Journal::from_spans(day.spans.clone(), reading.params.clone());
        journal.learn(lessons);
        for hint in &reading.hints {
            journal.apply(hint.clone());
        }
        let end = day.spans.last().map_or(Time(0), |s| s.end);
        // The day as it first looks, before the user corrects anything.
        println!("== {}, before any corrections", day.label);
        print!("{}", render(&journal.timeline(end), reading.detail));
        match day.hints.len() {
            0 => println!("Corrections made: none\n"),
            _ => {
                println!("Corrections made:");
                for hint in &day.hints {
                    println!("  {}", timechopper::ui::describe_hint(hint));
                }
                println!();
            }
        }
    }
    Ok(())
}

fn demo(name: Option<&str>, log: bool, reading: &Reading) -> Result<(), String> {
    if name == Some("week") {
        return week(reading);
    }
    let chosen: Vec<_> = scenarios::ALL
        .iter()
        .filter(|s| name.is_none_or(|n| n == s.name))
        .collect();
    if chosen.is_empty() {
        let names: Vec<_> = scenarios::ALL.iter().map(|s| s.name).collect();
        return Err(format!(
            "no such scenario; try one of: {}",
            names.join(", ")
        ));
    }
    for (i, scenario) in chosen.iter().enumerate() {
        let spans = (scenario.spans)();
        let marks = (scenario.marks)();
        if log {
            print!("{}", timechopper::live::log_text(&spans, &marks));
            continue;
        }
        if i > 0 {
            println!();
        }
        println!("== {}: {}", scenario.name, scenario.about);
        print_timeline(&spans, &marks, &[], reading);
    }
    Ok(())
}

/// The live window, fed by a replay or by the watcher.
fn live(opts: UiOptions) -> Result<(), String> {
    use std::sync::{Arc, OnceLock};
    use timechopper::live::{self, Clock};
    use timechopper::ui::Live;

    // The feed wakes the window when something arrives; the window's
    // context only exists once the window does.
    let context: Arc<OnceLock<eframe::egui::Context>> = Arc::default();
    let waker = context.clone();
    let wake = move || {
        if let Some(ctx) = waker.get() {
            ctx.request_repaint();
        }
    };
    let mut journal = Journal::new(opts.params.clone());
    let (clock, feed, log): (Arc<dyn Clock>, _, _) = match &opts.replay {
        Some(name) => {
            let scenario = scenarios::ALL
                .iter()
                .find(|s| s.name == name)
                .ok_or_else(|| {
                    let names: Vec<_> = scenarios::ALL.iter().map(|s| s.name).collect();
                    format!("no such scenario; try one of: {}", names.join(", "))
                })?;
            let (clock, feed) =
                live::replay((scenario.spans)(), (scenario.marks)(), opts.speed, wake);
            (clock, feed, None)
        }
        None => {
            let (clock, feed, path) = watch(&opts, Box::new(wake))?;
            let (log, earlier) =
                live::LogFile::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            journal.learn(learn(&path, &opts.params));
            (clock, feed, Some((log, earlier)))
        }
    };
    // Software rendering (no GPU, e.g. in a VM) is most likely to work with
    // wgpu on Windows and with OpenGL elsewhere; if the first choice can't
    // start, try the other.
    let (log, earlier) = match log {
        Some((log, earlier)) => (Some(log), earlier),
        None => (None, Vec::new()),
    };
    let mut live = Live::new(journal, feed, clock, log);
    live.resume(earlier);
    let app = std::rc::Rc::new(std::cell::RefCell::new(Some(live)));
    let renderers = match cfg!(windows) {
        true => [eframe::Renderer::Wgpu, eframe::Renderer::Glow],
        false => [eframe::Renderer::Glow, eframe::Renderer::Wgpu],
    };
    let mut failures = Vec::new();
    for renderer in renderers {
        let viewport = eframe::egui::ViewportBuilder::default()
            .with_title("timechopper")
            .with_inner_size([400.0, 620.0])
            .with_min_inner_size([300.0, 260.0])
            .with_always_on_top();
        let options = eframe::NativeOptions {
            viewport,
            renderer,
            ..Default::default()
        };
        let (shared, context) = (app.clone(), context.clone());
        let started = eframe::run_native(
            "timechopper",
            options,
            Box::new(move |cc| {
                let _ = context.set(cc.egui_ctx.clone());
                let live = shared.borrow_mut().take().ok_or("started twice")?;
                Ok(Box::new(live))
            }),
        );
        match started {
            Ok(()) => return Ok(()),
            Err(e) => failures.push(format!("{renderer:?}: {e}")),
        }
        if app.borrow().is_none() {
            break; // it ran, and then failed: don't start it again
        }
    }
    Err(failures.join("; "))
}

type Feed = (
    std::sync::Arc<dyn timechopper::live::Clock>,
    std::sync::mpsc::Receiver<timechopper::live::Observation>,
    std::path::PathBuf,
);

#[cfg(windows)]
fn watch(opts: &UiOptions, wake: Box<dyn Fn() + Send>) -> Result<Feed, String> {
    win::follow(opts, wake)
}

#[cfg(not(windows))]
fn watch(_opts: &UiOptions, _wake: Box<dyn Fn() + Send>) -> Result<Feed, String> {
    Err("watching windows is only supported on Windows; try --replay <SCENARIO>".into())
}

#[cfg(windows)]
fn run(opts: Options) -> ExitCode {
    win::run(opts)
}

#[cfg(not(windows))]
fn run(_opts: Options) -> ExitCode {
    eprintln!("timechopper: watching windows is only supported on Windows");
    ExitCode::FAILURE
}
