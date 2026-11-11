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
only been tried on synthetic logs. There is no UI; the interpretation has
the calls one would need.

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
   Of the rest, a window is identified by the part that, for its app,
   changes least from one visit to the next: the project rather than each
   file, the site rather than each page. A part seen only once in passing
   identifies nothing; such a window is identified by its app alone.
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

### Seeing what was swept in

A row hides what it is made of, and the guess is weakest exactly there. So
each row is followed by `?` lines for what the guess is least sure of, and
`--contents` lists everything in it:

```
09:00-09:19  19m55s  report.txt  [Notepad, Mozilla Firefox]  (8 visits)
                   · Notepad · report.txt        16m  4 visits
                   · Mozilla Firefox           3m15s  3 visits
                   ? Mozilla Firefox · Gmail     40s  1 visit  - seen once, in an app the task uses; also a task of its own
```

Two kinds of doubt are raised:

- **A member that looks like a thing of its own**: it has an identity
  (a recurring title part) and is also the main activity of another task, or
  turns up in several other tasks. Mail and chat look like this.
- **A possible split**: a time at which the row's subject changes for good
  (`? 09:33  timechopper may give way to billing here`), but too much else
  carried on across it for the guess to split there.

`demo webmail`, `demo email-twice` and `demo buried-switch` show each.

### Correcting the guess

Corrections are *hints*: they refer to the log by time and by pattern, so
they keep their meaning while the log is still growing.

```
--apart notepad                     Notepad is a task of its own, running
                                    alongside whatever it alternates with
--apart firefox:gmail               ... so is mail in the browser
--apart firefox@09:09:30-09:10:10   ... or just the browser between those times
--together :timechopper :billing    these are one task, not two
--lens code=app                     ignore Visual Studio Code's titles
--split 09:33                       the task in focus at 09:33 is a different
                                    task from then on (accepts a possible split)
--join 09:14:30 09:00               the row in focus at 09:14:30 belongs to
                                    the task in focus at 09:00
--name 09:00 "Ticket 12"            what to call the task in focus at 09:00;
                                    tasks with the same name are one task
--max-gap 60                        an hour away is still an interruption
```

`--apart`, `--together` and `--lens` change how the guess is made, so they
can shift other rows too. `--split`, `--join` and `--name` are applied to
the result and change nothing but the rows they are aimed at.

### For a UI

There is no UI yet, but the calls one would make exist
(`src/interpret/journal.rs`), with the log allowed to be still in progress:

- `Journal::observe(at, window)` on every focus change;
  `Journal::timeline(now)` interprets everything up to the present moment,
  the window in focus included. Re-reading a log of 5,000 focus changes
  takes about 20 ms, so this can be called on every redraw.
- On the timeline being shown, `split(row, at)`, `join(row, into)`,
  `separate(row, member, scope)`, `name(row, name)` and `confirm(row)` turn
  "do this to that row" into a hint. `separate` takes a scope: just here, or
  every window like it.
- `Journal::preview(hint, now)` returns the timeline that would result and
  the rows that would change; `apply`, `undo` and `retract` edit the list of
  hints.
- `confirm` freezes a row exactly as it stands. Without it, a row can still
  change as the day goes on: an app seen once is an interruption until it is
  seen again.

`--until 09:30` shows any log as it stood at that time.

### Known limits

- An interruption is recognised by its *app*. Mail read in a tab of a
  browser the task also uses is counted with the task. It is flagged only if
  mail was also read on its own, or in several tasks, that day.
- An app visited twice or more during a task is taken to be part of it; the
  same flag applies.
- A change of project is only offered as a possible split, not made, when
  more time goes into a shared app than into the projects themselves.
- Time away from the computer is only known if the log says so (`-`); the
  watcher does not detect idleness.
- Standing rules ("mail is always its own task") are not kept between logs.

The code is in `src/interpret/`; `tests/interpret.rs` and `tests/editing.rs`
show each behaviour on a synthetic log.

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
