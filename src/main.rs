//! timechopper: watch the focused (foreground) window and print what can be
//! learned about it: window title, class, owning process, its executable and
//! its command line.

#[cfg(windows)]
mod win;

use std::process::ExitCode;

const USAGE: &str = "\
Usage: timechopper [--once] [--exit-after <SECONDS>]

Watches the foreground window and prints information about it every time
focus moves to another window or the focused window's title changes.

Options:
  --once                  Print the current foreground window and exit
  --exit-after <SECONDS>  Stop watching after this many seconds
  -h, --help              Show this help
";

pub struct Options {
    pub once: bool,
    pub exit_after: Option<std::time::Duration>,
}

fn parse_args() -> Result<Options, String> {
    let mut opts = Options {
        once: false,
        exit_after: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--once" => opts.once = true,
            "--exit-after" => {
                let v = args.next().ok_or("--exit-after needs a value")?;
                let secs: f64 = v
                    .parse()
                    .ok()
                    .filter(|s: &f64| s.is_finite() && *s >= 0.0)
                    .ok_or_else(|| format!("invalid number of seconds: {v}"))?;
                opts.exit_after = Some(std::time::Duration::from_secs_f64(secs));
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(opts)
}

fn main() -> ExitCode {
    let opts = match parse_args() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("timechopper: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    run(opts)
}

#[cfg(windows)]
fn run(opts: Options) -> ExitCode {
    win::run(opts)
}

#[cfg(not(windows))]
fn run(_opts: Options) -> ExitCode {
    eprintln!("timechopper: only Windows is supported");
    ExitCode::FAILURE
}
