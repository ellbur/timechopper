//! timechopper: watch the focused (foreground) window and print what can be
//! learned about it; interpret a log of focus changes as a timeline of tasks.

#[cfg(windows)]
mod win;

use std::process::ExitCode;

use timechopper::interpret::{self, Hint, Lens, Params, Pattern, Span, model, render, scenarios};

const USAGE: &str = "\
Usage: timechopper [--once] [--exit-after <SECONDS>]
       timechopper summarize <LOG> [INTERPRETATION OPTIONS]
       timechopper demo [<SCENARIO>] [--log] [INTERPRETATION OPTIONS]

With no command, watches the foreground window (Windows only) and prints
information about it every time focus moves to another window or the focused
window's title changes.

  --once                  Print the current foreground window and exit
  --exit-after <SECONDS>  Stop watching after this many seconds
  -h, --help              Show this help

summarize reads a focus log (one line per focus change: HH:MM:SS APP TITLE,
with APP '-' when nothing is in use) and prints the tasks it seems to show.
demo does the same for built-in synthetic logs; --log prints the log itself.

Interpretation options, to correct the guess:
  --apart <PATTERN>               These windows are a task of their own
  --together <PATTERN> <PATTERN>  These windows belong to the same task
  --lens <APP>=<auto|app|title>   How much of this app's titles tells its
                                  windows apart
  --max-gap <MINUTES>             Longest interruption a task carries on
                                  through (default 15)
A PATTERN is APP, APP:TEXT or :TEXT, where TEXT is part of a window title.
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
}

#[derive(Default)]
struct Reading {
    hints: Vec<Hint>,
    params: Params,
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
            "--apart" => {
                let pattern = Pattern::parse(&value("a pattern")?)?;
                self.hints.push(Hint::Apart(pattern));
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
    let command = args.next_if(|a| a == "summarize" || a == "demo");
    let mut opts = Options {
        once: false,
        exit_after: None,
    };
    let mut reading = Reading::default();
    let mut positional = None;
    let mut log = false;
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

fn print_timeline(spans: &[Span], reading: &Reading) {
    let timeline = interpret::interpret(spans, &reading.hints, &reading.params);
    println!(
        "{} focus changes, {} rows:\n",
        spans.len(),
        timeline.sessions.len()
    );
    print!("{}", render::render(&timeline));
}

fn summarize(path: &str, reading: &Reading) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let events = model::parse_log(&text).map_err(|e| format!("{path}: {e}"))?;
    print_timeline(&model::spans(&events), reading);
    Ok(())
}

fn demo(name: Option<&str>, log: bool, reading: &Reading) -> Result<(), String> {
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
        if log {
            print!("{}", model::format_log(&spans));
            continue;
        }
        if i > 0 {
            println!();
        }
        println!("== {}: {}", scenario.name, scenario.about);
        print_timeline(&spans, reading);
    }
    Ok(())
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
