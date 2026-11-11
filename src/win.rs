//! Windows implementation: a WinEvent hook on foreground changes plus
//! queries for the window's title/class and the owning process's details.

use std::cell::RefCell;
use std::ffi::c_void;
use std::process::ExitCode;

use windows::Wdk::System::Threading::{NtQueryInformationProcess, PROCESSINFOCLASS};
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, HWND, LPARAM, STATUS_BUFFER_OVERFLOW, STATUS_BUFFER_TOO_SMALL,
    STATUS_INFO_LENGTH_MISMATCH, UNICODE_STRING,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, DispatchMessageW, EVENT_OBJECT_NAMECHANGE, EVENT_SYSTEM_FOREGROUND,
    EnumChildWindows, GetClassNameW, GetForegroundWindow, GetMessageW, GetWindowTextLengthW,
    GetWindowTextW, GetWindowThreadProcessId, KillTimer, MSG, OBJID_WINDOW, SetTimer,
    TranslateMessage, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_TIMER,
};
use windows::core::{BOOL, PWSTR};

use crate::Options;

/// `ProcessCommandLineInformation` (Windows 8.1+). Not exported by name.
const PROCESS_COMMAND_LINE_INFORMATION: PROCESSINFOCLASS = PROCESSINFOCLASS(60);

pub fn run(opts: Options) -> ExitCode {
    let fg = unsafe { GetForegroundWindow() };
    report("focused", fg);
    if opts.once {
        return ExitCode::SUCCESS;
    }

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
        if fg_hook.is_invalid() {
            eprintln!("timechopper: SetWinEventHook failed");
            return ExitCode::FAILURE;
        }

        let timer = opts.exit_after.map(|d| {
            let ms = d.as_millis().clamp(1, u32::MAX as u128) as u32;
            SetTimer(None, 0, ms, None)
        });

        // WinEvent callbacks for out-of-context hooks are delivered while
        // this thread pumps messages.
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_TIMER && msg.hwnd.is_invalid() {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        if let Some(t) = timer {
            let _ = KillTimer(None, t);
        }
        let _ = UnhookWinEvent(fg_hook);
        if !name_hook.is_invalid() {
            let _ = UnhookWinEvent(name_hook);
        }
    }
    ExitCode::SUCCESS
}

thread_local! {
    /// The last (hwnd, title) printed, to avoid repeating identical reports.
    static LAST: RefCell<Option<(isize, String)>> = const { RefCell::new(None) };
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
        _ => {}
    }
}

fn report(what: &str, hwnd: HWND) {
    let stamp = timestamp();
    if hwnd.is_invalid() {
        println!("[{stamp}] {what}: (no foreground window)\n");
        return;
    }
    let title = window_title(hwnd);
    let key = (hwnd.0 as isize, title.clone());
    let repeated = LAST.with(|l| l.borrow().as_ref() == Some(&key));
    if repeated {
        return;
    }
    LAST.with(|l| *l.borrow_mut() = Some(key));

    let (pid, tid) = window_pid_tid(hwnd);
    let class = window_class(hwnd);
    let mut proc_pid = pid;
    let mut exe = process_image(pid);

    let mut out = String::new();
    out += &format!("[{stamp}] {what}\n");
    out += &format!("  hwnd:    {:#x}\n", hwnd.0 as usize);
    out += &format!("  title:   {title:?}\n");
    out += &format!("  class:   {class}\n");

    // UWP apps are hosted by ApplicationFrameHost.exe; the real app's
    // process owns a child window.
    if exe
        .as_ref()
        .is_ok_and(|p| file_name_is(p, "ApplicationFrameHost.exe"))
        && let Some(child_pid) = uwp_app_pid(hwnd, pid)
    {
        out += &format!("  host:    pid {pid}, {}\n", exe.as_deref().unwrap_or("?"));
        proc_pid = child_pid;
        exe = process_image(child_pid);
    }

    out += &format!("  pid:     {proc_pid} (thread {tid})\n");
    out += &format!("  exe:     {}\n", or_error(exe));
    out += &format!("  cmdline: {}\n", or_error(process_command_line(proc_pid)));
    println!("{out}");
}

fn or_error(r: Result<String, String>) -> String {
    r.unwrap_or_else(|e| format!("<unavailable: {e}>"))
}

fn file_name_is(path: &str, name: &str) -> bool {
    path.rsplit(['\\', '/'])
        .next()
        .is_some_and(|f| f.eq_ignore_ascii_case(name))
}

fn timestamp() -> String {
    let t = unsafe { GetLocalTime() };
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
fn uwp_app_pid(frame: HWND, host_pid: u32) -> Option<u32> {
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
