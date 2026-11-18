# System-Ping Open Source

[Deutsche Fassung: README.md](README.md)

A small network latency monitor for Windows, Linux and macOS: ping several targets at an interval,
keep a history per target, live chart, statistics, status list, alerts and CSV export.

Built as a clean-room re-implementation: the interface follows screenshots of a commercial tool,
the code is entirely independent (Rust + [egui/eframe]).

The interface is bilingual: **German and English**, switchable in the settings under “Language”
(or once per run with `--lang en`). Every text comes from a single table (`app/src/i18n.rs`) and
the choice is saved together with the other options.

## Download

The [latest release](https://github.com/ajoe/System-Ping/releases/latest) has a ready file for each
of the three systems. GitHub Actions builds them — one runner per operating system, since a program
file for one system cannot be produced on another — for every tag: `v0.9.1` gives the files below
(`.github/workflows/release.yml`).

| File | |
| --- | --- |
| `System-Ping-<version>-macos-arm64.zip` | `System-Ping.app` for Apple Silicon: double click, no terminal window |
| `system-ping-<version>-macos-arm64.tar.gz` | the same program file without a bundle, for the command line (`--help`) |
| `System-Ping-<version>-windows-x64.zip` | `System-Ping.exe` (file icon and version info inside the file) and `icon.ico` |
| `system-ping-<version>-linux-x64.tar.gz` | the program file for the command line (x86_64) |
| `SHA256SUMS.txt` | checksums of every file |

**macOS** files are ad-hoc signed rather than notarised by Apple, so a file downloaded from the net
carries the system's quarantine flag. Move the app to `/Applications` before the first start —
launched from the Downloads folder, a quarantined app gets relocated to a random read-only place —
and if the system refuses to open it, use right click → Open or
`xattr -dr com.apple.quarantine /Applications/System-Ping.app`. The same applies to the bare binary
from a browser download (`xattr -d com.apple.quarantine <file>`); files fetched with `curl` are
unaffected.

**Windows** needs the Visual C++ runtime, which a current Windows has anyway; the `.exe` is portable
and keeps its state folder next to itself.

**Linux** is built on Ubuntu 24.04 against the system libraries (X11 and Wayland bindings load at
run time, OpenGL through EGL/GLX). A desktop install has those; otherwise
`sudo apt install libegl1 libgl1 libx11-6 libxcursor1 libxi6 libxinerama1 libxrandr2
libxkbcommon0 libxkbcommon-x11-0`. There is no `.desktop` file and no AppImage: make it executable
and run it.

None of this touches data: state and history stay in the system's state folder (see “State and
files").

For the newest build of `main` rather than the newest version: the current Mac bundle sits on the
pre-release page [`releases/tag/dev`](https://github.com/ajoe/System-Ping/releases/tag/dev) — no
version in the file name, replaced by every push, and therefore without a checksums file.

## Building

A normal Rust toolchain setup (`rustup`) is required — the MSVC build on Windows. On Linux, besides
the compiler you also need the X11/Wayland development packages, because `eframe` binds directly
against the system libraries:

```sh
# Debian/Ubuntu
sudo apt install build-essential pkg-config libgl1-mesa-dev libx11-dev libxcursor-dev \
                 libxi-dev libxinerama-dev libxrandr-dev libwayland-dev libxkbcommon-dev
# Fedora
sudo dnf install gcc mesa-libGL-devel libX11-devel libXcursor-devel libXi-devel \
                 libXinerama-devel libXrandr-devel wayland-devel libxkbcommon-devel
```

macOS only needs the command line tools (`xcode-select --install`). On Windows,
`app/.cargo/config.toml` holds nothing but a suppression filter for a misleading linker warning from
`ring`; the file can be deleted and the project builds unchanged.

```sh
cd app
cargo build --release        # -> app/target/release/system-ping(.exe)
cargo test                   # 127 tests (47 unit + 80 integration, no GUI)
cargo clippy --all-targets   # clean
./macos/bundle.sh            # macOS only: -> app/target/macos/System-Ping.app (see below)
```

Those same gates run without anyone typing them: `ci.yml` checks every push and pull request on
Windows, Linux and macOS, and each of those jobs builds a program file that paints one frame
(`--shot`) — a binary that cannot get a window is not buildable, however well it compiles. Every one
of those runs builds the Mac bundle too (`System-Ping.app`, see “macOS: a bundle instead of a bare
program”) and uploads it as an artefact of the run, so that shape is tested by a push and not by a
release alone. The newest of those bundles also sits on a standing pre-release page,
[`releases/tag/dev`](https://github.com/ajoe/System-Ping/releases/tag/dev): the same file, replaced
by every push, with no version in its name and no expiry date. `release.yml` does the same for the
release files (see “Download").

Other systems can be checked by hand (type check only, without binding their libraries):

```sh
rustup target add x86_64-apple-darwin x86_64-unknown-linux-gnu
cargo check --target x86_64-unknown-linux-gnu --profile test --lib   # no warnings
```

The tests need no internet: they ping `127.0.0.1` with the system's own `ping`. A machine that
drops ICMP to its own loopback (some firewalls, some build environments) cannot run the one test
that lets the live loop actually measure — it says so in its output and skips itself.

### What differs per platform

Everything a given OS does differently is fenced off in `app/src/platform/` — the rest of the app
never asks which system it is on:

| | Windows | Linux | macOS |
| --- | --- | --- | --- |
| state folder | next to the `.exe` (portable) | `$XDG_DATA_HOME/system-ping`, else `~/.local/share/system-ping` | `~/Library/Application Support/system-ping` |
| ping call | `ping -n 1 -w <ms>` | `ping -c 1 -W <s>` | `ping -c 1 -W <ms>` |
| gateway guess | `route print`, else `ipconfig` | `ip route show`, else `netstat -rn` | `route -n get default`, else `netstat -rn` |
| autostart | `HKCU\...\Run` | not offered | not offered |
| file icon | embedded in the `.exe` | `.desktop` entry (not implemented) | `.icns` in a bundle (`app/macos/bundle.sh`) |

Missing features answer `false`/`None` instead of pretending: on Linux and macOS the autostart
button in the top bar and the checkbox in the settings do not appear at all, and the settings screen
explains that the feature is unsupported there instead. `SYSTEM_PING_STATE_DIR` overrides the folder
on every system.

### The executable's icon

Only Windows knows an icon embedded inside the program file: `app/build.rs` attaches the resources
at build time — `assets/icon.ico` as the file icon (16/32/48/64/256 px) plus a version block. The
[`winresource`] crate turns that into a `.res`; the normal linker embeds it, so no extra step and no
post-processing of the `.exe` is needed. On Linux and macOS `build.rs` does nothing, because there
the icon travels as a file next to the program.

```sh
cargo build --release   # the .res appears in app/target/release/build/system-ping/*/out/
```

The file icon of the macOS bundle is made from `assets/icon.png` at the same time:
`app/macos/bundle.sh` turns it into an `icon.icns` with `sips` and `iconutil` (16/32/64/128/256 px,
nothing above 256 px invented by upscaling) and puts it in `System-Ping.app/Contents/Resources/`.

The window icon at runtime is independent of that and embedded directly in `main.rs`
(`assets/icon.png`), so `cargo run` shows an icon on every system too.

[`winresource`]: https://crates.io/crates/winresource

### macOS: a bundle instead of a bare program

`target/release/system-ping` is a Mach-O file, which to macOS is an executable in the same sense as
a shell script. Double clicking it in Finder, or putting `open` on it, therefore does not bring up
the program's window but Terminal: Terminal.app has claimed exactly that file type as its “Shell”
role and runs the program inside itself. A terminal window that opens at launch is thus not the
program's doing — it never starts a shell itself (the `windows_subsystem` switch in `main.rs` only
concerns Windows).

A Mac starts the program properly as a bundle, and `app/macos/bundle.sh` builds one from the finished
binary — with nothing beyond the command line tools, and with nothing in the program changed:

```sh
cd app
cargo build --release          # or: ./macos/bundle.sh, which builds it as part of the run
./macos/bundle.sh              # -> app/target/macos/System-Ping.app
open target/macos/System-Ping.app
```

What the script assembles in `app/target/macos/` is the folder a double click in Finder opens: the
program in `Contents/MacOS/`, the `Contents/Info.plist` made from `macos/Info.plist` with the name
and version taken from `Cargo.toml`, that `icon.icns` next to it, an ad-hoc signature, and a report
to the LaunchServices service so the new icon does not take a while to reach the Dock. `--no-build`
takes the binary that is already there when it has to be quick.

None of this touches the program: on macOS the state folder stays
`~/Library/Application Support/system-ping` (`app/src/platform/statedir.rs`), so both ways of
starting it find the same targets, history and window place. To install it,
`cp -R app/target/macos/System-Ping.app /Applications` is enough.

## Using it

The program is called `system-ping` (`system-ping.exe` on Windows). The examples below are written
the Windows way; on Linux and macOS read `./target/release/system-ping` in their place.

```
system-ping.exe                        open the window
system-ping.exe --help                 overview of all arguments (alias: -h)
system-ping.exe --lang <de|en>         interface language for this run, without saving it
system-ping.exe --export-csv <file>    export the stored history as CSV and exit
system-ping.exe --screenshot <file.png> [view]   render a picture of the interface, as the test asks for it (alias: --shot)
system-ping.exe --shot <png> [view]    render a picture of the interface (chart|stats|events|settings|edit)
system-ping.exe --theme <light|dark>   override the look for this session
system-ping.exe --collect <seconds>    open the window briefly, measure, print numbers, exit
```

`--help` answers in the language of the run: German without `--lang`, English with `--lang en`.
This is the program's own output, not a transcription:

```
System-Ping Open Source — latency monitor

Without arguments the window opens with the last state (targets, settings, window size).
Measurements are written to the state folder automatically every 8 s.

Arguments:
  --help, -h              show this overview and exit
  --lang <de|en>          interface language for this run, without saving it
  --theme <light|dark>    force the look for this run, without saving it
  --collect <seconds>     measure in the window, report and exit (self test)
  --export-csv [file]     write the stored measurements as CSV and exit
                          (without a file name: default folder, timestamp in the name)
  --shot <file.png> [view]
                          render a picture of the interface and exit; views: chart,
                          events, stats, settings, edit, plus a horizon such as 24h

Environment variables:
  SYSTEM_PING_STATE_DIR   force the state folder (files, protocol, default export)
  SYSTEM_PING_MAX_FRAMES  abort the frame loop after n pictures (test runs only)
```

The picture can also show a different time window without touching the saved settings:
`--shot day.png 24h` (also “12 h”, “30 min”, “90 s”). The view and the window come directly after
the file, so `--shot pic.png stats 6h --theme dark`; an option word such as `--theme` otherwise
ends the list of views — or the picture would land on the chart.

Targets are created in the settings panel (name + host). The default gateway is guessed from the
system's own routing table on the first start (`route print`/`ipconfig`, `ip route show`,
`route -n get default`). A **click** on a target in the left list opens the inline
editor: name, address, category and whether it is measured. Enter or “✓ Save” commits, Escape or
“✗ Cancel” discards. The editor also shows the target's details (measurements, mean, min/max,
losses in the visible time window). A second click on the same target leaves the editor standing
together with what was typed; only cancel, Escape, save or clicking another row close it — and a
pending edit is saved first. **Remove** sits in the editor's top left (“remove”), i.e. at the
target one is working on — no longer in the top right at the selected entry.

Adding a target happens in the bar above the list — two fields, Enter as confirmation. Deleting
the history is now **“Clear measurements”** under Data and expressly affects all targets. Under
**“Info”** stand the version and the git commit this build was made from — in a bug report, that
answers which program exactly is meant.

Every target gets its own line colour automatically, from a palette of eight; in the chart, the
legend, the list on the left and the sparklines it is always the same one. The colour button in the
editor changes it: choose a colour, or take the automatic one back with “Auto”. What you choose is
stored as a palette number and survives a restart; a target that was chosen by hand gives its
colour back to the automatic ones. When there are more targets than colours, colours are reused.
The small disc in front of the name is unaffected — it still shows the state
(green/yellow/orange/red).

**Your own order:** grab a row in the left list and hold it (left click, keep the button down),
then drag it up or down. The list rearranges itself under the pointer straight away, the dragged
row is drawn faded, the cursor shows the grabbing hand, and a grip of six dots on the left edge
marks the row. Letting go decides the final place — above another row's middle you land in front
of it, below its middle behind it, and in the gap between two rows exactly at that boundary. The
order is the one written to the state file, so it survives a restart. A target whose editor is
open cannot be dragged: the form has priority, and a drop onto it is deliberately refused — other-
wise it would hit one of the form's fields. Whether a press on a row is a click or a pull is egui's
own call, not ours: the hand may wander up to six points from where it pressed and still count as a
click, so dragging never opens an editor by accident and a click that slips a little never moves
the list. `app/tests/drag_order.rs` checks both cases with two *different* points for press and
release; one of them (`a_drag_released_far_from_where_it_began_...`) exists because pressing and
releasing at the same point is exactly what let this gesture ship broken once.

## Language: one table, two languages

Every text of the interface hangs in `app/src/i18n.rs`: one entry in German and one in English per
text, grouped by the place where it appears. From that table a macro generates one method per text
(`lang.chart()`, `lang.tip_sort_fmt()`), and every place in the program asks for its text through
it — which is why switching languages needs no restart and no rebuild: the next pass simply asks
again.

Deliberately **not** translated are things that are data rather than interface: the protocol file
and the event lines it writes stay German (an export that relabels its own past when a switch is
flipped would be hard to read), as does the CSV header (`timestamp,target,latency_ms,status`) and
the placeholder texts in the address fields. The category and view names from
`TargetKind::label()` / `View::label()` stayed German too — they serve as stable identifiers in the
protocol and the screenshot arguments; what is shown comes from `Lang::kind()` and `Lang::view()`.

Adding a text means: write an arm in `texts!` (one for `de`, one for `en`) **and** add the name to
the list in `accessors!`. If the name is missing there, the method is missing; if it is entered
twice, the build fails — both with an error message rather than silently.

## Text: always the same black or white

Every text follows one rule: black in the light look, **white** in the dark one — across labels,
buttons, headings, input fields, axis labels and the chart drawn by the painter. egui ships this
differently (its light buttons are grey 60, its labels grey 80, in the dark look grey 140 and
180), so `theme::apply` writes the one colour into all five widget states; strong text is thereby
truly white or black rather than just a lighter grey.

What should look quieter — hidden targets, axes, hint text — is the same black or white with less
strength, never a grey of its own. Only two things are deliberately coloured: the accents (outage
red, warning amber, good green, a target's own line colour) and selected text, which needs the
opposite colour on a blue highlight. `app/tests/textcolor.rs` checks the rule.

One egui quirk has to be kept in mind: `RichText::weak()` sets the colour **inside the text**,
before the widget paints — the colour taken from the widget only fills holes. A weakly written
selectable label (the view buttons above, the category chips) therefore stayed grey while the
“▶ Start all” button next to it was correctly black. That is why such buttons in `ui::view_text`
carry no `weak()`: what is active is said by the selection background and not by paler type.

Symbols in the strings are a case of their own, because none of the fonts egui ships has them on its
own: Ubuntu-Light, the proportional font behind every word, has letters, umlauts and dashes — and no
▶. Since egui 0.37 the icon fonts (`NotoEmoji`, `emoji-icon-font`) sit behind the
`monochrome_emoji_fonts` feature, and eframe does not switch it on, so it is set on the `egui` entry
of `app/Cargo.toml`. What was still missing after that were the arrows of the shortcut table, the ●
of “Live” and the ◉ of a marked row: those are only in Hack, the monospace font.
`theme::install_fonts` appends Hack to the end of the proportional family, so those characters are
answered by the monospace font while every letter stays Ubuntu-Light. The characters no shipped font
has at all — ✕ ✓ ✗ ⌂ — are out of the strings and replaced by × ✔ ✖ 📁. `app/tests/fonts.rs` reads
every non-ascii character of the strings in `src` and compares its glyph with the `.notdef` picture:
a character without a glyph shows up in the test, not in the window.

The window does **not** start measuring on its own — only “▶ Start all” (or the single button per
target) fills the charts. `--collect` does it automatically and is the way to check the measurement
without looking at the screen:

```
$ app\target\release\system-ping.exe --collect 10
collect: Gateway          192.168.178.1       samples=5 replies=5 last=1.0 ms
collect: DNS (Google)     8.8.8.8             samples=5 replies=5 last=13.0 ms
collect: DNS (Cloudflare) 1.1.1.1             samples=5 replies=5 last=10.0 ms
collect: 15 measurements in this time, 15 of them replies
```

The run saves its result, so `--export-csv` shows exactly those values right afterwards.

## Seeing what you want to measure

Whether a target is in the chart, the legend, the event list and the statistics is decided in one
place: it has to pass the text filter above (which searches name, address **and** category) and its
category has to be on.

* **Categories as switches.** The second row of the header holds one button per category in use
  (“Gateway”, “Local network”, “DNS”, …). Switched off, it hides every target of that category in
  all views — measuring continues, only the presentation is missing. The “all” button brings
  everything back; the choice is saved.
* **Single targets by clicking the legend.** Clicking a target's name in the legend at the top of
  the chart hides exactly that one target, a second click brings it back. Hidden targets stay in
  the legend — written weakly and with a hollow colour blob instead of a full disc — otherwise
  there would be no way back; the “◉” button at the right end of the same row switches the same
  target back on. They are missing from chart, status list and statistics table, and the totals
  row does not count them.
* **Nothing in the selection** is not an empty screen: the chart says what causes it (text filter,
  all targets hidden, category off), the statistics table writes “No target in the selection”.

## Paging and zooming the chart

Besides “◀ back”/“forward ▶” and the jump buttons to outliers (⚑), the chart itself can be driven:

* **Mouse wheel over the chart** zooms into the time window and back out again, centred on the spot
  under the pointer. The result is always one of the presets from the list above (5 s to 24 h), so
  the axis stays a name the combo box can show too. Zooming leaves “live”.
* **Double-click in the chart** goes back to the end: the window is set to “live”.

## How the chart paints: lines or dots

Two buttons sit at the right edge of the chart's heading, **“Lines”** and **“Dots”**. They answer
two different questions, which is why there are both:

* **Lines** (the default) connects each target's measurements into one curve. It shows *how* the
  latency develops — a slow climb, a step after a reboot.
* **Dots** paints every measurement as a dot of its own, and its size says how far the value stands
  from the **average of that target** — measured against the target's own spread. Values near the
  average stay specks; individually they are not interesting, the average has already said what
  they mean. What lies far out, above or below, is painted larger and in the colour the statistics
  use for an outlier.

Behind this: the problem with a connection is its outliers, and those are rare. In a curve that is
made mostly of the boring values between them, they disappear — as dots of different sizes they
stand out on their own. The same number the measurements table and the orange rings use decides
which ones: more than three standard deviations above the average of the target's own series. The
mode is saved together with the other settings.

## Keyboard

The shortcuts are printed on the view buttons and listed in the settings under “Keyboard”. They
only fire when no input field is active and no modifier key is held — otherwise they would do
exactly the wrong thing in a text field.

| Key | Effect |
| --- | --- |
| `1` `2` `3` `4` | chart · status list · statistics · settings |
| `space` | measure all targets or pause |
| `←` / `→` | page back / forward in the time window (a quarter of the window) |
| `Esc` | cancel an open editor, otherwise back to the current window (“live”) |
| `Enter` | add a new target while an address field has the focus |
| mouse wheel | over the chart: enlarge / shrink the time window |
| double-click | on an outage row: the chart jumps to that time |

## Measuring: interval and time window

The distance between two probes is not a free number but a step from `0.25 · 0.5 · 1 · 2 · 5 ·
10 s`. The bar above holds the only control at runtime, and the settings show the same value as a
readable line — two sliders for one thing were just an opportunity to contradict each other. A
stored or typed value lands on the nearest step.

The chart's time window is a preset (5 s to 24 h), not a slider; the log slider has moved out of
the header and now sits there as a preset list up to 24 h, where it belongs.

Minimising does not interrupt the measuring. Scheduling the probes and collecting their results
happens in `App::logic`, and `eframe` calls that method even when it is not painting the window —
so a minimised window keeps measuring just as a visible one does. What arrives in the meantime
carries the time of the program clock and fills the chart without a hole as soon as the window is
back on screen; only the painting waits for the window. `app/tests/minimised.rs` drives the app
the way a minimised window is driven — `logic` in a loop, `ui` never — and checks that the curve
stays unbroken.

## Statistics table

* **Sorting** by clicking a column header (↑/↓ show the direction). The first click sorts
  descending — “the worst first” —, the second turns it around. Only the order of the list is
  sorted: every target keeps its id and with it its colour.
* The **“Total” row** sums only the targets currently in the selection (text filter, category
  switches, hidden targets). Median and p95 come from **all replies together**, not as the mean of
  the medians — otherwise the row would state a latency no single target ever measured. A target
  without a reply does not pull the minimum to 0; with no reply at all, the minimum reads “—”.
* The table scrolls horizontally when not all columns fit the width.

## A chart over 24 hours

From two hours on, the axes are labelled with the time of day, otherwise with minutes and seconds.

So that a day is not a hole in the chart, the number of measurements a target keeps grows
automatically with the window: “Max. samples per target” therefore stands at **auto** (0). At 24 h
and one second apart that is about 260,000 values per target (roughly 4 MiB); whoever types a
number gets it, anything below 1,024 is raised to that floor. The sparklines in the left list cut
off at the p95 boundary of their own series (at least 40 ms), otherwise a single outlier would
flatten every other curve into a straight line.

## Event list

Every row carries the time `[HH:MM:SS]` (the same computation as in the CSV export), and the top
line reads “N of M” — only what belongs in the protocol is kept, but everything is counted. The
**“outages only”** switch hides recoveries and informational lines; a **double-click** on an outage
row jumps the chart to that moment. In the chart the list is embedded as a compact version below
the curve; in its own view it fills the whole space and starts at the oldest entry.

## Status bar

The counter “saved N s ago” says when the state file was last written (autosave runs every 8 s); a
separate “Save” button would be a third way for something that happens automatically anyway.
`Runtime` switches to `H:MM:SS` from one hour on. The **“CSV”** button exports straight into the
state folder and opens it; “⌂ Data” in the header does nothing else than show that folder window.

Two kinds of sentences share this one line: what a click reported (“3 → 1” after a move, “target
saved”, which path an export is writing) and what announced itself (a measurement, a protocol file
that can no longer be written, a save that did not go through, the autostart answer from the
registry). Whatever the user did last
wins — and stays winning, not just for one frame. The automatic warning is not thrown away in the
process: it comes back as soon as nothing newer is in the way, because it is still true; a protocol
file that gets retried every 30 s must not be allowed to bury an action for half a minute. The
reason this needed a rule at all: while nothing was being measured the bar kept its previous text,
so a click with measuring stopped looked like no click at all. `app/tests/fixes.rs` checks this.

## State and files

* State folder: **next to the program file** on Windows (portable), `$XDG_DATA_HOME/system-ping`
  or `~/.local/share/system-ping` on Linux, and `~/Library/Application Support/system-ping` on
  macOS. On every system it can be forced with the environment variable `SYSTEM_PING_STATE_DIR`.
* `system-ping-state.json` — options plus history. Version 2 stores every measurement with an
  absolute Unix time in milliseconds; version 1 files (seconds since process start) are lifted onto
  a real time axis when loaded.
* That file is **read part by part and written atomically**. A part that cannot be read — an
  `options` block with a field this version does not know, a history entry that is not an entry —
  costs only itself, and the rest of the file still counts. A file bigger than 32 MiB is not read at
  all (that is a mistyped state folder rather than one's own settings), and a history entry that
  nests deeper than the format ever writes is dropped on its own. Writes go through a sibling file
  and a rename, so a crash or a power loss halfway through a save leaves the previous, complete file
  where the next start looks for it. A save that fails for a different reason — the file held open by
  another program, a state folder that is not there — says so in the status bar (“Could not save:
  …”), and the “saved … s ago” clock stops advancing: it should only confirm a file that really
  exists. `app/tests/save_status.rs` checks both cases.
* The same file remembers the **window size and position** (field `window`, inner position in
  physical pixels, size in logical points). On the next start the window lies where it was;
  implausible values (zero or tiny) are discarded and the default size 1080×700 stands. A `--shot`
  does not change the stored position.
* The same file also remembers the **language** (field `language`, `de` or `en`). A `--lang` on the
  command line is valid only for that run and writes nothing back.
* CSV export: `timestamp,target,latency_ms,status` with local ISO-8601 stamps. “Export CSV” writes
  into the state folder, “Save as…” asks for a destination first; both run in the background so the
  window does not stall while answering. The status bar names the file one is waiting for, and
  later its result. An optional protocol file writes every measurement on continuously, right away.
  A target name that starts with `=`, `+`, `-` or `@` is written with a leading `'` — otherwise a
  spreadsheet turns that cell into a formula, and target names are free input. Quotes and line
  breaks inside a name are escaped, so one measurement stays one line.

## What is still missing

Three fields are stored in the state file but deliberately have no button because nothing is behind
them: sound on outage/recovery, tray icon, start minimised. The values are kept in case one of the
features arrives later.

## Pictures

`app/shots/` holds one current screen per language (German dark, English light), produced with
`--shot`:

```sh
system-ping.exe --shot app\shots\de-chart.png chart
system-ping.exe --lang en --theme light --shot app\shots\en-chart.png chart
```

## Folders

* `app/` — the crate (sources in `app/src`, including `app/src/platform/` for everything
  OS-specific, GUI-free tests in `app/tests`, `build.rs` for the Windows resources, `macos/` for the
  Mac bundle (`bundle.sh` and `Info.plist`), screenshots in `app/shots`)
* `assets/` — `icon.ico` (file icon) and `icon.png` (window icon at runtime)
* `.github/workflows/` — `ci.yml` (gates on Windows, Linux and macOS for every push; the Mac bundle
  as an artefact of the run) and
  `release.yml` (the release files for all three systems, built on a tag)
* `reference/` — screenshots of the original and own comparison pictures
* `capture/` — notes about the reference material
