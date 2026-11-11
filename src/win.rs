//! Windows implementation: a WinEvent hook on foreground changes plus
//! queries for the window's title/class and the owning process's details.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

use windows::Wdk::System::Threading::{NtQueryInformationProcess, PROCESSINFOCLASS};
use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, HWND, LPARAM, STATUS_BUFFER_OVERFLOW, STATUS_BUFFER_TOO_SMALL,
    STATUS_INFO_LENGTH_MISMATCH, UNICODE_STRING,
};
use windows::Win32::System::SystemInformation::{GetLocalTime, GetTickCount};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, DispatchMessageW, EVENT_OBJECT_DESTROY, EVENT_OBJECT_NAMECHANGE,
    EVENT_SYSTEM_FOREGROUND, EnumChildWindows, GetClassNameW, GetForegroundWindow, GetMessageW,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, KillTimer, MSG, OBJID_WINDOW,
    SetTimer, TranslateMessage, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_TIMER,
};
use windows::core::{BOOL, PWSTR};

use timechopper::interpret::{Event, Mark, Marker, Record, Time, Window};
use timechopper::live::{Clock, FnClock, Observation};

use crate::{Feed, Options, UiOptions};

/// `ProcessCommandLineInformation` (Windows 8.1+). Not exported by name.
const PROCESS_COMMAND_LINE_INFORMATION: PROCESSINFOCLASS = PROCESSINFOCLASS(60);

/// How long to keep looking for a UWP app's window inside its frame.
const UWP_RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);
const UWP_RETRY_INTERVAL_MS: u32 = 100;

pub fn run(opts: Options) -> ExitCode {
    let fg = unsafe { GetForegroundWindow() };
    if opts.once {
        report_once(fg);
        return ExitCode::SUCCESS;
    }
    report("focused", fg);
    pump(opts.exit_after, None)
}

/// Watches on a thread of its own, sending each focus change (and each time
/// the user goes away or comes back) to the live window.
pub fn follow(opts: &UiOptions, wake: Box<dyn Fn() + Send>) -> Result<Feed, String> {
    let path = match &opts.log_file {
        Some(path) => path.clone(),
        None => {
            let dir = std::env::var_os("LOCALAPPDATA")
                .ok_or("LOCALAPPDATA is not set; give --log-file")?;
            let t = unsafe { GetLocalTime() };
            let name = format!("{:04}-{:02}-{:02}.log", t.wYear, t.wMonth, t.wDay);
            PathBuf::from(dir).join("timechopper").join(name)
        }
    };
    let (send, receive) = mpsc::channel();
    let idle = opts.idle;
    std::thread::spawn(move || {
        SINK.with(|s| *s.borrow_mut() = Some(Sink { send, wake }));
        report("focused", unsafe { GetForegroundWindow() });
        pump(None, Some(idle));
    });
    let clock: Arc<dyn Clock> = Arc::new(FnClock(|| time_of_day(&unsafe { GetLocalTime() })));
    Ok((clock, receive, path))
}

/// Where observations go instead of being printed, on a thread that feeds
/// the live window.
struct Sink {
    send: Sender<Observation>,
    wake: Box<dyn Fn() + Send>,
}

fn time_of_day(t: &SYSTEMTIME) -> Time {
    Time::hms(t.wHour.into(), t.wMinute.into(), t.wSecond.into())
}

/// Hooks focus and title changes and pumps messages until `exit_after` (if
/// given), checking every second whether the user has been away for `idle`.
fn pump(exit_after: Option<Duration>, idle: Option<Duration>) -> ExitCode {
    unsafe {
        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        let fg_hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_event),
            0,
            0,
            flags,
        );
        let name_hook = SetWinEventHook(
            EVENT_OBJECT_NAMECHANGE,
            EVENT_OBJECT_NAMECHANGE,
            None,
            Some(on_event),
            0,
            0,
            flags,
        );
        // Windows being closed (of all the windows on the desktop: only
        // those that have had focus are reported).
        let destroy_hook = SetWinEventHook(
            EVENT_OBJECT_DESTROY,
            EVENT_OBJECT_DESTROY,
            None,
            Some(on_event),
            0,
            0,
            flags,
        );
        if fg_hook.is_invalid() {
            eprintln!("timechopper: SetWinEventHook failed");
            return ExitCode::FAILURE;
        }

        let exit_timer = exit_after.map(|d| {
            let ms = d.as_millis().clamp(1, u32::MAX as u128) as u32;
            SetTimer(None, 0, ms, None)
        });
        let retry_timer = SetTimer(None, 0, UWP_RETRY_INTERVAL_MS, None);
        let idle_timer = idle.map(|_| SetTimer(None, 0, 1000, None));

        // WinEvent callbacks for out-of-context hooks are delivered while
        // this thread pumps messages.
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_TIMER && msg.hwnd.is_invalid() {
                if Some(msg.wParam.0) == exit_timer {
                    break;
                }
                if msg.wParam.0 == retry_timer {
                    retry_pending();
                    continue;
                }
                if let (Some(timer), Some(idle)) = (idle_timer, idle)
                    && msg.wParam.0 == timer
                {
                    check_idle(idle);
                    continue;
                }
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        flush_pending();
        if let Some(t) = exit_timer {
            let _ = KillTimer(None, t);
        }
        let _ = KillTimer(None, retry_timer);
        if let Some(t) = idle_timer {
            let _ = KillTimer(None, t);
        }
        let _ = UnhookWinEvent(fg_hook);
        if !name_hook.is_invalid() {
            let _ = UnhookWinEvent(name_hook);
        }
        if !destroy_hook.is_invalid() {
            let _ = UnhookWinEvent(destroy_hook);
        }
    }
    ExitCode::SUCCESS
}

/// A UWP frame whose app window hasn't been attached yet (this happens for a
/// moment after an app starts or is restored). Reporting waits for it.
struct Pending {
    stamp: SYSTEMTIME,
    what: String,
    hwnd: HWND,
    deadline: Instant,
}

thread_local! {
    /// The last (hwnd, title) reported, to avoid repeating identical reports.
    static LAST: RefCell<Option<(isize, String)>> = const { RefCell::new(None) };
    static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) };
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
    /// Whether the user is away (no input for a while) as far as the live
    /// window has been told.
    static AWAY: Cell<bool> = const { Cell::new(false) };
    /// Windows that have had focus, as last seen: to say which was closed
    /// (a closed window can no longer be asked).
    static SEEN: RefCell<HashMap<isize, Window>> = RefCell::new(HashMap::new());
    /// Whether anything has been reported yet: the window in focus when
    /// watching starts was not opened then.
    static STARTED: Cell<bool> = const { Cell::new(false) };
}

/// Tells the live window the user has gone away, or come back, when the
/// time since the last keyboard or mouse input crosses `idle`.
fn check_idle(idle: Duration) {
    let mut info = LASTINPUTINFO {
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
        return;
    }
    let quiet = Duration::from_millis(unsafe { GetTickCount() }.wrapping_sub(info.dwTime).into());
    match (AWAY.get(), quiet >= idle) {
        (false, true) => {
            flush_pending();
            AWAY.set(true);
            // Away since the last input, not since we noticed.
            let now = time_of_day(&unsafe { GetLocalTime() });
            let at = Time(now.0.saturating_sub(quiet.as_secs()));
            send(Record::Focus(Event { at, window: None }));
            LAST.with(|l| *l.borrow_mut() = None);
        }
        (true, false) => {
            AWAY.set(false);
            report("back", unsafe { GetForegroundWindow() });
        }
        _ => {}
    }
}

/// Sends an observation to the live window, if this thread feeds one.
/// Returns whether it did.
fn send(observation: Observation) -> bool {
    SINK.with(|s| match s.borrow().as_ref() {
        Some(sink) => {
            let _ = sink.send.send(observation);
            (sink.wake)();
            true
        }
        None => false,
    })
}

/// Reports focus on `hwnd` (or on nothing): to the live window, or printed.
fn emit(stamp: &SYSTEMTIME, what: &str, hwnd: HWND) {
    if SINK.with(|s| s.borrow().is_some()) {
        // While away, focus moving by itself is not the user's doing.
        if !AWAY.get() {
            let window = (!hwnd.is_invalid()).then(|| window_of(hwnd));
            let at = time_of_day(stamp);
            if let Some(window) = &window {
                // A window seen for the first time has just been opened.
                let new = SEEN.with(|s| s.borrow_mut().insert(hwnd.0 as isize, window.clone()));
                if new.is_none() && STARTED.get() {
                    let mark = Mark::Opened(window.clone());
                    send(Record::Mark(Marker { at, mark }));
                }
            }
            STARTED.set(true);
            send(Record::Focus(Event { at, window }));
        }
        return;
    }
    if !hwnd.is_invalid() {
        SEEN.with(|s| s.borrow_mut().insert(hwnd.0 as isize, window_of(hwnd)));
    }
    let stamp = format_stamp(stamp);
    match hwnd.is_invalid() {
        true => println!("[{stamp}] {what}: (no foreground window)\n"),
        false => println!("{}", describe(&stamp, what, hwnd)),
    }
}

/// The app (executable file name) and title of a window, as the
/// interpretation sees it.
fn window_of(hwnd: HWND) -> Window {
    let mut pid = window_pid_tid(hwnd).0;
    if is_uwp_frame(hwnd)
        && let Some(app) = uwp_app_pid(hwnd)
    {
        pid = app;
    }
    let app = match process_image(pid) {
        Ok(path) => path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string(),
        Err(_) => "unknown".to_string(),
    };
    Window {
        app,
        title: window_title(hwnd),
    }
}

unsafe extern "system" fn on_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    if hwnd.is_invalid() || id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32 {
        return;
    }
    match event {
        EVENT_SYSTEM_FOREGROUND => report("focus changed", hwnd),
        EVENT_OBJECT_NAMECHANGE if hwnd == unsafe { GetForegroundWindow() } => {
            report("title changed", hwnd)
        }
        EVENT_OBJECT_DESTROY => closed(hwnd),
        _ => {}
    }
}

/// A window was destroyed: report it if it is one that has had focus.
fn closed(hwnd: HWND) {
    let Some(window) = SEEN.with(|s| s.borrow_mut().remove(&(hwnd.0 as isize))) else {
        return;
    };
    let stamp = timestamp();
    let at = time_of_day(&stamp);
    let mark = Mark::Closed(window.clone());
    if !send(Record::Mark(Marker { at, mark })) {
        println!(
            "[{}] window closed: {:?}\n",
            format_stamp(&stamp),
            window.title
        );
    }
}

fn report(what: &str, hwnd: HWND) {
    let stamp = timestamp();
    if hwnd.is_invalid() {
        flush_pending();
        emit(&stamp, what, hwnd);
        return;
    }
    let key = (hwnd.0 as isize, window_title(hwnd));
    if LAST.with(|l| l.borrow().as_ref() == Some(&key)) {
        return;
    }
    LAST.with(|l| *l.borrow_mut() = Some(key));

    // Anything still waiting is superseded by this event; print what we had.
    flush_pending();

    if is_uwp_frame(hwnd) && uwp_app_pid(hwnd).is_none() {
        let deadline = Instant::now() + UWP_RESOLVE_TIMEOUT;
        let what = what.to_owned();
        PENDING.with(|p| {
            *p.borrow_mut() = Some(Pending {
                stamp,
                what,
                hwnd,
                deadline,
            })
        });
        return;
    }
    emit(&stamp, what, hwnd);
}

fn retry_pending() {
    let ready = PENDING.with(|p| {
        p.borrow()
            .as_ref()
            .is_some_and(|p| Instant::now() >= p.deadline || uwp_app_pid(p.hwnd).is_some())
    });
    if ready {
        flush_pending();
    }
}

fn flush_pending() {
    if let Some(p) = PENDING.with(|p| p.borrow_mut().take()) {
        emit(&p.stamp, &p.what, p.hwnd);
    }
}

/// `--once`: report the current window, waiting briefly if it is a UWP frame
/// whose app window isn't attached yet.
fn report_once(hwnd: HWND) {
    let stamp = format_stamp(&timestamp());
    if hwnd.is_invalid() {
        println!("[{stamp}] focused: (no foreground window)\n");
        return;
    }
    if is_uwp_frame(hwnd) {
        let deadline = Instant::now() + UWP_RESOLVE_TIMEOUT;
        while uwp_app_pid(hwnd).is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(UWP_RETRY_INTERVAL_MS.into()));
        }
    }
    println!("{}", describe(&stamp, "focused", hwnd));
}

fn describe(stamp: &str, what: &str, hwnd: HWND) -> String {
    let title = window_title(hwnd);
    let class = window_class(hwnd);
    let (pid, tid) = window_pid_tid(hwnd);
    let host_exe = process_image(pid);

    let mut out = String::new();
    out += &format!("[{stamp}] {what}\n");
    out += &format!("  hwnd:    {:#x}\n", hwnd.0 as usize);
    out += &format!("  title:   {title:?}\n");
    out += &format!("  class:   {class}\n");

    // UWP apps are hosted by ApplicationFrameHost.exe; the real app's
    // process owns a child window of the frame.
    let mut proc_pid = pid;
    let mut exe = host_exe;
    if is_host_exe(&exe) {
        out += &format!("  host:    pid {pid}, {}\n", exe.as_deref().unwrap_or("?"));
        match uwp_app_pid(hwnd) {
            Some(app_pid) => {
                proc_pid = app_pid;
                exe = process_image(app_pid);
            }
            None => out += "  app:     <not found: no app window inside the frame>\n",
        }
    }

    out += &format!("  pid:     {proc_pid} (thread {tid})\n");
    out += &format!("  exe:     {}\n", or_error(exe));
    out += &format!("  cmdline: {}\n", or_error(process_command_line(proc_pid)));
    out
}

fn is_host_exe(exe: &Result<String, String>) -> bool {
    exe.as_ref()
        .is_ok_and(|p| file_name_is(p, "ApplicationFrameHost.exe"))
}

fn is_uwp_frame(hwnd: HWND) -> bool {
    is_host_exe(&process_image(window_pid_tid(hwnd).0))
}

fn or_error(r: Result<String, String>) -> String {
    r.unwrap_or_else(|e| format!("<unavailable: {e}>"))
}

fn file_name_is(path: &str, name: &str) -> bool {
    path.rsplit(['\\', '/'])
        .next()
        .is_some_and(|f| f.eq_ignore_ascii_case(name))
}

fn timestamp() -> SYSTEMTIME {
    unsafe { GetLocalTime() }
}

fn format_stamp(t: &SYSTEMTIME) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
    )
}

fn window_title(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd).max(0) as usize;
        let mut buf = vec![0u16; len + 1];
        let n = GetWindowTextW(hwnd, &mut buf).max(0) as usize;
        String::from_utf16_lossy(&buf[..n])
    }
}

fn window_class(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) }.max(0) as usize;
    String::from_utf16_lossy(&buf[..n])
}

fn window_pid_tid(hwnd: HWND) -> (u32, u32) {
    let mut pid = 0u32;
    let tid = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    (pid, tid)
}

/// Find a child window of a UWP frame owned by a process other than the host.
fn uwp_app_pid(frame: HWND) -> Option<u32> {
    let (host_pid, _) = window_pid_tid(frame);
    struct Search {
        host_pid: u32,
        found: Option<u32>,
    }
    unsafe extern "system" fn cb(child: HWND, lparam: LPARAM) -> BOOL {
        let s = unsafe { &mut *(lparam.0 as *mut Search) };
        let (pid, _) = window_pid_tid(child);
        if pid != 0 && pid != s.host_pid {
            s.found = Some(pid);
            return BOOL(0);
        }
        BOOL(1)
    }
    let mut s = Search {
        host_pid,
        found: None,
    };
    unsafe {
        let _ = EnumChildWindows(
            Some(frame),
            Some(cb),
            LPARAM(&mut s as *mut Search as isize),
        );
    }
    s.found
}

/// A process handle that is closed on drop.
struct Process(HANDLE);

impl Process {
    fn open(pid: u32) -> Result<Self, String> {
        if pid == 0 {
            return Err("no process id".into());
        }
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            .map(Process)
            .map_err(|e| format!("OpenProcess: {}", e.message()))
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn process_image(pid: u32) -> Result<String, String> {
    let p = Process::open(pid)?;
    let mut buf = vec![0u16; 1024];
    loop {
        let mut len = buf.len() as u32;
        let r = unsafe {
            QueryFullProcessImageNameW(p.0, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
        };
        match r {
            Ok(()) => return Ok(String::from_utf16_lossy(&buf[..len as usize])),
            Err(_) if buf.len() < 32768 => buf.resize(buf.len() * 4, 0),
            Err(e) => return Err(format!("QueryFullProcessImageNameW: {}", e.message())),
        }
    }
}

fn process_command_line(pid: u32) -> Result<String, String> {
    let p = Process::open(pid)?;
    unsafe {
        let mut len = 0u32;
        let status = NtQueryInformationProcess(
            p.0,
            PROCESS_COMMAND_LINE_INFORMATION,
            std::ptr::null_mut(),
            0,
            &mut len,
        );
        if status.is_ok() || len == 0 {
            return Err(format!(
                "NtQueryInformationProcess: status {:#x}",
                status.0 as u32
            ));
        }
        if ![
            STATUS_INFO_LENGTH_MISMATCH,
            STATUS_BUFFER_TOO_SMALL,
            STATUS_BUFFER_OVERFLOW,
        ]
        .contains(&status)
        {
            return Err(format!(
                "NtQueryInformationProcess: status {:#x}",
                status.0 as u32
            ));
        }
        // u64 elements keep the UNICODE_STRING header suitably aligned.
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let status = NtQueryInformationProcess(
            p.0,
            PROCESS_COMMAND_LINE_INFORMATION,
            buf.as_mut_ptr() as *mut c_void,
            len,
            &mut len,
        );
        if status.is_err() {
            return Err(format!(
                "NtQueryInformationProcess: status {:#x}",
                status.0 as u32
            ));
        }
        let us = &*(buf.as_ptr() as *const UNICODE_STRING);
        if us.Buffer.is_null() || us.Length == 0 {
            return Ok(String::new());
        }
        let chars = std::slice::from_raw_parts(us.Buffer.0, us.Length as usize / 2);
        Ok(String::from_utf16_lossy(chars))
    }
}
