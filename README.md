# timechopper
Watches the focused (foreground) window on Windows and prints what can be
learned about it each time focus moves or the focused window's title changes:

- window handle, title and class
- owning process id and thread id
- the process's executable path
- the process's full command line

UWP apps (hosted by `ApplicationFrameHost.exe`) are resolved to the real app
process. If the app window is not attached to its frame yet (for a moment
after an app starts), the report waits up to 5 seconds for it.

## Usage

```
timechopper                    # watch until Ctrl+C
timechopper --once             # print the current foreground window and exit
timechopper --exit-after 30    # watch for 30 seconds
```

Some details may be unavailable for processes this one can't open (for
example, elevated or protected processes when running unelevated); they are
shown as `<unavailable: ...>`.
