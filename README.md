# timechopper

A helper for time tracking: it notices which window you are using, so that
when you forget to write down that you switched tasks, you can go back and
work out when you did and what you were doing.

It has two parts:

- a **watcher** (Windows) that reports the focused window every time it
  changes, and
- an **interpretation** (any platform) that boils a log of those changes
  down to a short list of tasks.

They are not connected yet: the watcher prints, and the interpretation has
only been tried on synthetic logs.

## Interpreting a focus log

```
timechopper demo                 # interpret the built-in synthetic logs
timechopper demo messy-day       # ... one of them
timechopper demo messy-day --log # print the log itself
timechopper summarize day.log    # interpret a log file
```

A log has one line per focus change, `HH:MM:SS APP TITLE`, with `-` for APP
when nothing is in use:

```
09:00:00 notepad.exe report.txt - Notepad
09:03:00 firefox.exe How to structure a quarterly report — Mozilla Firefox
09:04:30 notepad.exe *report.txt - Notepad
09:08:30 -
```

Ten focus changes between Notepad, a browser and (once) Outlook come out as:

```
09:00-09:24  23m50s  report.txt  [Notepad, Mozilla Firefox]  (9 visits, 40s elsewhere)
                     also: How to structure a quarterly report 2m45s · Quarterly report examples 1m · ...
    09:14-09:15     40s  Inbox  [Outlook]

Totals
  23m50s  report.txt  [Notepad, Mozilla Firefox]
     40s  Inbox  [Outlook]
  24m30s  tracked
```

Each row is a *session* of a task. The time shown is what was spent on it;
an indented row is carved out of the row above, whose time does not include
it. Every second of the log is counted in exactly one row, so the times add
up however the grouping is changed.

### How it guesses

1. **What a window is.** A title is split into parts (`main.rs`,
   `timechopper`, `Visual Studio Code`). The part naming the app is dropped.
   Of the rest, a window is identified by the part that keeps coming back
   and that its app spends the most time on: the project rather than each
   file, the site rather than each page. A window with no such part is
   identified by its app alone.
2. **What goes together.** Windows the user keeps going back and forth
   between are one task. The sequence of visits is split where nothing
   before a point is returned to after it. Within such a stretch, a visit to
   an app used nowhere else in it is an interruption: a task of its own,
   carved out of the one around it, which carries on as a single session. A
   stretch is also split where its main activity changes for good while
   others (a browser, a terminal) stay in use across the change.
3. **When.** A task left for more than 15 minutes and picked up again gets a
   new session ("part 2 of 2"); a shorter absence is an interruption within
   one session.
4. **What to call it.** A session is named after the title part that had
   focus the longest, whichever window it came from, so a browser's page
   titles can explain what an `Untitled` Notepad window was for.

### Correcting the guess

The grouping is a guess, and several readings of the same log can be
reasonable. Hints change the reading; there is no UI for them yet, only
command-line options:

```
--apart notepad                     Notepad is a task of its own, running
                                    alongside whatever it alternates with
--apart :gmail                      ... so is any window with "gmail" in its title
--together :timechopper :billing    these are one task, not two
--lens code=app                     ignore Visual Studio Code's titles
--lens firefox=title                tell every Firefox page apart
--max-gap 60                        an hour away is still an interruption
```

### Known limits

- An interruption is recognised by its *app*. Email read in a browser tab,
  in a browser the task also uses, is counted with the task unless a hint
  sets it apart.
- An app visited twice or more during a task is taken to be part of it. Two
  looks at Outlook during one stretch of work are counted with that work;
  `--apart outlook` keeps email separate.
- Time away from the computer is only known if the log says so (`-`); the
  watcher does not detect idleness.

The code is in `src/interpret/`; `tests/interpret.rs` shows each behaviour on
a synthetic log.

## Watching the focused window (Windows)

Prints what can be learned about the focused (foreground) window each time
focus moves or the focused window's title changes:

- window handle, title and class
- owning process id and thread id
- the process's executable path
- the process's full command line

UWP apps (hosted by `ApplicationFrameHost.exe`) are resolved to the real app
process. If the app window is not attached to its frame yet (for a moment
after an app starts), the report waits up to 5 seconds for it.

```
timechopper                    # watch until Ctrl+C
timechopper --once             # print the current foreground window and exit
timechopper --exit-after 30    # watch for 30 seconds
```

Some details may be unavailable for processes this one can't open (for
example, elevated or protected processes when running unelevated); they are
shown as `<unavailable: ...>`.
