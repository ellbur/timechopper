# timechopper

A helper for time tracking: it notices which window you are using, so that
when you forget to write down that you switched tasks, you can go back and
work out when you did and what you were doing.

It has three parts:

- a **watcher** (Windows) that notices every time the focused window
  changes,
- an **interpretation** (any platform) that boils a log of those changes
  down to a short list of tasks, and
- a **live window** that connects the two: a small panel in the corner of
  the screen showing the day so far, grouped into tasks, up to the window in
  focus now, where the grouping can be corrected as you go.

## The live window

```
timechopper ui                                  # Windows: watch, and show the day so far
timechopper ui --replay messy-day --speed 120   # any platform: replay a synthetic day
```

It opens a 400×620 window that stays on top, in the bottom right corner of
the screen. From the top:

- **Now**: the clock, the window in focus, and the row it is part of, with
  how long that has gone on (and the task's total for the day). Below, a
  note box with *Mark* and *Break*: mark the moment, with a note or
  without, or the start of a break. *Undo* takes back the last marker or
  correction; hover over it to see which.
- **Rows**: newest first, each with its time, the time actually spent on it
  and what it seems to be about. Carve-outs are indented under the row they
  interrupted. A row that just changed shape (a window joined it, or a
  correction moved it) flashes; the row in focus has an outline; `? 2`
  means two things in it may not belong. Click a row to open it:
  - each thing swept into it, with its time and visits (hover for why it
    is there); `?` marks the doubtful ones, and *own task…* takes one out,
    *just here* or *wherever it shows up*;
  - possible splits, each with *split here*;
  - the markers made during it, each with *split here*, and the one it
    starts at, with *use as name* (markers show under a row even when it is
    closed);
  - *Name…*, *Confirm* (keep the row exactly as it is), *Join into…*
    another row's task, and *Split* at a time typed in.

  Hovering over any of these outlines in red the rows it would change.
- **Log**: focus changes, newest first, each in the colour of the task it
  went to (click one to open its row), with markers between them: yours,
  and windows opened and closed.

### Markers

A marker is something that happened at a moment: you noted something,
started a break, a window was opened or closed. It is **not** a task. Notes
are not taken to mean "this went on from here", and a note doesn't have to
start anything. A marker is evidence of where one task may have ended:

- Where the work itself changes, but not clearly enough for the guess to
  split it (it would only offer the split, `?`), a note, a break or a
  closed window at that moment (within 2 minutes) settles it: the row is
  split there. The new row says which marker it starts at.
- Where the work doesn't change, a marker cuts nothing. It is shown in the
  row it falls in, for you to go back to when you reconstruct the day, with
  *split here* if it was a change after all.
- A window being opened counts for nothing, since new windows open all the
  time within a task. It is only shown in the log.

A cut a marker made can be undone like any other (*Join into…*).

The window re-reads the whole day every second and on every focus change.
Each focus change, marker and correction is appended to a log file as it
happens, by default
`%LOCALAPPDATA%\timechopper\YYYY-MM-DD.log` (`--log-file` to choose).
Starting again on the same day carries on from it: the rows come back as
you left them, and *Undo* still takes back what you did before the restart. Without keyboard or mouse
input for 5 minutes (`--idle`), you are counted as away from when the input
stopped. Clicking in the live window itself doesn't count as a focus change:
the window you were in keeps the time.

Not yet:

- The day ends at midnight only by restarting: times are times of day.
- There is no tray icon and no way to collapse the window to a single line.

The code is in `src/ui.rs` (the window), `src/live.rs` (clock, replay, log
file) and `src/win.rs` (`follow`: the watcher feeding the window).
`tests/ui.rs` drives the window headlessly: focus changes arrive and buttons
are clicked through the accessibility tree.

### Parts of a task

Some clients want to know how the time on one task was spent: so much
drafting the brief, so much research for it. Drafting and research
alternate all day, so as rows they would be dozens of snippets each.
Instead, a task can be divided into **parts**: shares of its time, by which
windows were in use, wherever that time fell. The rows stay as they are;
under each one a line gives its time in each part, and the totals give the
task's:

```
09:48-11:29   1h41m  "Smith v. Jones brief": ...  (part 2 of 2, 37 visits)
                   = Drafting 1h14m · Research 26m59s
...
   2h09m  "Smith v. Jones brief": ...
   1h14m    Drafting
  54m59s    Research
```

Open a row to:

- **Divide by app**: one part per app (Word, the browser). Click a part's
  name to rename it ("Drafting", "Research"). Each thing in the row has a
  menu with the part it counts towards: move it to another part, or to a
  part of its own. Anything left out of every part counts as "other".
- **Combine with…** another row's task, when the guess made two tasks of
  what you count as one (research in the morning, drafting after lunch):
  one task, each keeping its time as a part.
- **Undivide** to put it back.

Parts always add up to the task's time. A division is one correction (`@hint
parts…` in the log), so *Undo* takes back a whole step. A task you divided
the same way on earlier days is offered that division ("? divided into
Drafting, Research on Mon"), and divided without asking after three days in
a row, like names (see below). `timechopper demo brief` is the example above;
`--combine` and `--parts` do the same from the command line.

### What earlier days teach

Corrections are made at times of day, which mean nothing the next day, but
they are about windows, which come back. When it starts, the live window
re-reads the earlier logs in its folder (the latest 20), each with its own
corrections, and learns two kinds of rule:

- **Names.** A task you named was mostly about some project or site (its
  identity: the activity with a subject that took the most time). Work on
  the same thing another day is probably called the same.
- **Separations.** Something you took out of a row (*own task…*) is a thing
  of its own, done on the side: mail, chat.
- **Divisions.** A task you divided into parts is divided the same way.

A rule from earlier days is evidence, not an order:

- After one day it is **offered** on the row: `? called "Billing" on Mon`
  with *use*; a window swept into a row says "taken out of its row on Mon".
  A name that changes from day to day (a ticket number) is only offered
  once the same name has held two days running.
- After **three days in a row** it is **applied** without asking, and says
  so ("named as on Mon, Tue, Wed", with *not this*; "own task, as on …").
  Something you said was a task of its own *wherever it shows up* is applied
  from the next day.
- Your corrections today always win. Renaming, taking a name away (*not
  this*), or joining a carved-out row back counts against the rule: it
  starts again from there.

Nothing is stored apart from the logs: the rules are worked out from them
each time, so undoing a correction on an earlier day changes what is
learned from it. `timechopper demo week` shows a synthetic week day by day;
`summarize --history` learns from the logs beside the one it reads.

Each day's window identity is still worked out from that day alone. A site
seen only once in a day has no identity that day, so nothing learned about
it applies.

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

Markers go between them: `HH:MM:SS @note TEXT`, `@break TEXT`,
`@opened APP TITLE`, `@closed APP TITLE`, and `@unmark HH:MM:SS` (a note or
break taken back). So do corrections made in the live window:
`@hint KIND<TAB>FIELDS…` (e.g. `@hint name<TAB>09:00:00<TAB>Q3 report`) and
`@unhint` (the latest one taken back); `summarize` applies them, before any
given on the command line. `timechopper demo marked-day` shows a day with notes and
a break, and `demo marked-day --log` shows its log.

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
4. **Markers.** Where step 2 would only offer a split, a note, a break or a
   closed window at that moment makes it. See [Markers](#markers).
5. **What to call it.** A session is named after the title part that had
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

The live window is built on these calls (`src/interpret/journal.rs`), which
allow the log to be still in progress:

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
- Time away from the computer is only known if the log says so (`-`). The
  live window writes that after 5 minutes without input; the plain watcher
  does not.
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
