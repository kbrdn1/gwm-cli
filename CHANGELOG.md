# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/), one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **A setting for which direction a mux pane opens in**
  ([#589](https://github.com/kbrdn1/gwm-cli/issues/589)). `[tui]
  mux_pane_direction` decides where the TUI's `t` key puts the worktree it
  opens: `"right"` (default) or `"down"` to split the current pane, `"window"`
  for a whole tmux window or zellij / herdr tab. It is also the direction a
  bare `gwm tmux|zellij|herdr <pattern> --split` takes, and the new
  `--direction right|down` overrides it for one invocation. The knob cycles
  live in the Settings panel under the **TUI** tab.

  Two directions rather than four: `left` and `up` reach tmux only through
  `split-window -b`, and herdr declares its `--direction` as `[possible
  values: right, down]`, so a fuller compass would be values one backend could
  not honour.

  One caveat, and it is the same shape as the herdr one below: under
  `"window"` a `[tui.macro*]` with `open_in = "mux_pane"` falls back to the
  PTY overlay on zellij, because `zellij action new-tab` takes no trailing
  command to run. The status bar names the backend that refused.

- **herdr is a third multiplexer backend**
  ([#588](https://github.com/kbrdn1/gwm-cli/issues/588)). `gwm herdr <pattern>`
  opens the matched worktree in a new [herdr](https://herdr.dev) tab, `-p`
  splits the current pane instead, and the TUI's `t` key finds herdr the way
  it finds tmux and zellij. Detection reads `$HERDR_ENV`, which herdr sets in
  every pane it manages, and it comes last in the cascade so nothing changes
  for a tmux or zellij user.

  Under the hood: `herdr tab create --workspace <id> --label <name> --cwd
  <path> --focus` and `herdr pane split --current --direction <right|down>
  --cwd <path> --focus`, verified against a live herdr 0.8.2 rather than its
  help text. The split needs a direction because herdr's parser has no default
  for one; which one it gets is the `mux_pane_direction` entry above. The other two flags are
  there because herdr's defaults are the opposite of what the names suggest:
  without `--focus` the tab opens where you cannot see it, and without
  `--workspace` it opens in whichever workspace the server had focused, which
  is another project's window as often as not.

  One surface stays on its old path: a `[tui.macro*]` with
  `open_in = "mux_pane"` still falls back to the PTY overlay under herdr, and
  now says so. A macro needs the new pane to run a command, and
  `herdr pane split` has no trailing-command form, so running one takes a
  second call with the pane id that `pane split` prints back.

- **A worktree note can be a checklist**
  ([#557](https://github.com/kbrdn1/gwm-cli/issues/557)). `Ctrl+t` in the note
  editor ticks the box on the line and spawns one when the line has none, from
  anywhere on the line; `Ctrl+u` makes the line a list item or takes the marker
  back off it; `Enter` continues the list and ends it on an empty item, the way
  every Markdown editor does. Both chords are Ctrl-modified because an
  unmodified printable is text in that modal, and which chord is left over is
  tmux's call: `Ctrl+b` is its prefix, and `Ctrl+h` / `Ctrl+j` / `Ctrl+k` /
  `Ctrl+l` are the vim-tmux-navigator pane set that tmux forwards only to a
  pane running vim.

  Ticking used to mean arrowing onto the right column and retyping a character
  by hand, which is what a note becomes after a day: "what to check before
  opening the PR" is a list you tick off.

- **A vim normal mode for the note editor**
  ([#557](https://github.com/kbrdn1/gwm-cli/issues/557)). `N` opens in normal
  mode: `hjkl`, `w` / `b` / `e` and their `W` / `B` / `E`, `0` / `^` / `$`,
  `gg` / `G`, `x`, `dd`, and `i` / `I` / `a` / `A` / `o` / `O` to enter
  insert. `o` and `O` carry the list marker the way `Enter` does, the modal
  title carries a `NORMAL` / `INSERT` chip, and the modal's own last row
  leads with the mode as a reverse-video badge (the treatment the statusbar
  context anchor already wears) before listing the keys that mode takes,
  as does the statusbar behind it. A list too long for the row is cut with a `…` rather than
  clipped at the frame, which reads as a list that ends there.

  **The cost is `Esc`, so it gets its own line: it no longer writes and closes
  on the first press.** It leaves insert, and the second press saves.
  `[tui] note_vim = false` buys the single-press gesture back and returns the
  editor to the modeless one, where every printable is text, and it is a
  toggle in the Settings panel's TUI tab like the other two TUI booleans. No counts, no
  registers, no undo: this is a scratch buffer, and `Ctrl+e` still hands the
  file to the real vim. The verbs are hard-coded rather than bindable, so
  `[tui.keys.modal.note]` holds the same four verbs either way and an
  unmodified printable bound to one of them is still refused at load time.

### Changed

- **A tmux or zellij split now opens to the right by default**
  ([#589](https://github.com/kbrdn1/gwm-cli/issues/589)). Up to 1.9 a split
  carried no direction at all, so each backend answered for itself: `tmux
  split-window` fell back to `-v` and stacked the pane, `zellij action
  new-pane` took "the biggest available space", and herdr went right because
  gwm hardcoded it. All three now pass a direction, and it defaults to
  `right`: it is what the `--split` help has promised since it shipped ("a
  horizontal split of the current pane"), and the half that is actually free
  on a wide screen. Set `[tui] mux_pane_direction = "down"` to get the old
  tmux behaviour back.

### Fixed

- **The note column captions itself**
  ([#595](https://github.com/kbrdn1/gwm-cli/issues/595)). The column shipped
  with an empty header on the grounds that its marker is binary, which left
  the marker sitting under a blank caption immediately right of the two-slot
  `I/P` group, where it read as a third slot of that group rather than as its
  own column. It now carries the same glyph it marks rows with, and both moved
  from `≡` to the `nf-oct-markdown` glyph the Working Tree pane already paints
  on a `.md` file, since a note is one. The column stays conditional, so a
  user who never writes a note keeps the exact table they had before.

## Past releases

In reverse chronological order:

- [`1.9.0`](changelogs/1.9.0.md), 2026-08-16
- [`1.8.0`](changelogs/1.8.0.md), 2026-08-13
- [`1.7.1`](changelogs/1.7.1.md), 2026-08-12
- [`1.7.0`](changelogs/1.7.0.md), 2026-08-12
- [`1.6.1`](changelogs/1.6.1.md), 2026-08-04
- [`1.6.0`](changelogs/1.6.0.md), 2026-08-03
- [`1.5.0`](changelogs/1.5.0.md), 2026-07-26
- [`1.4.0`](changelogs/1.4.0.md), 2026-07-25
- [`1.3.0`](changelogs/1.3.0.md), 2026-07-24
- [`1.2.0`](changelogs/1.2.0.md), 2026-07-21
- [`1.1.1`](changelogs/1.1.1.md), 2026-07-16
- [`1.1.0`](changelogs/1.1.0.md), 2026-07-15
- [`1.0.3`](changelogs/1.0.3.md), 2026-07-09
- [`1.0.2`](changelogs/1.0.2.md), 2026-07-06
- [`1.0.1`](changelogs/1.0.1.md), 2026-07-01
- [`1.0.0`](changelogs/1.0.0.md), 2026-06-26
- [`0.9.0`](changelogs/0.9.0.md), 2026-06-07
- [`0.8.0`](changelogs/0.8.0.md), 2026-06-01
- [`0.7.0`](changelogs/0.7.0.md), 2026-05-23
- [`0.6.0`](changelogs/0.6.0.md), 2026-05-21
- [`0.5.0`](changelogs/0.5.0.md), 2026-05-20
- [`0.4.0`](changelogs/0.4.0.md), 2026-05-19
- [`0.3.0`](changelogs/0.3.0.md), 2026-05-19
- [`0.2.0`](changelogs/0.2.0.md), 2026-05-18
- [`0.1.0`](changelogs/0.1.0.md), 2026-05-18

### Pre-releases

Per-RC notes covering only the delta against the previous RC (or against the previous stable, for `rc.1`):

- [`0.10.0-rc.4`](changelogs/pre-releases/0.10.0-rc.4.md), 2026-06-17
- [`0.10.0-rc.3`](changelogs/pre-releases/0.10.0-rc.3.md), 2026-06-17
- [`0.10.0-rc.2`](changelogs/pre-releases/0.10.0-rc.2.md), 2026-06-16
- [`0.10.0-rc.1`](changelogs/pre-releases/0.10.0-rc.1.md), 2026-06-10
- [`0.9.0-rc.3`](changelogs/pre-releases/0.9.0-rc.3.md), 2026-06-07
- [`0.9.0-rc.2`](changelogs/pre-releases/0.9.0-rc.2.md), 2026-06-06
- [`0.9.0-rc.1`](changelogs/pre-releases/0.9.0-rc.1.md), 2026-06-02
- [`0.8.0-rc.5`](changelogs/pre-releases/0.8.0-rc.5.md), 2026-06-01
- [`0.8.0-rc.4`](changelogs/pre-releases/0.8.0-rc.4.md), 2026-05-29
- [`0.8.0-rc.3`](changelogs/pre-releases/0.8.0-rc.3.md), 2026-05-29
- [`0.8.0-rc.2`](changelogs/pre-releases/0.8.0-rc.2.md), 2026-05-23
- [`0.8.0-rc.1`](changelogs/pre-releases/0.8.0-rc.1.md), 2026-05-23
- [`0.7.0-rc.3`](changelogs/pre-releases/0.7.0-rc.3.md), 2026-05-23
- [`0.7.0-rc.2`](changelogs/pre-releases/0.7.0-rc.2.md), 2026-05-23
- [`0.7.0-rc.1`](changelogs/pre-releases/0.7.0-rc.1.md), 2026-05-22
- [`0.6.0-rc.1`](changelogs/pre-releases/0.6.0-rc.1.md), 2026-05-20
- [`0.5.0-rc.2`](changelogs/pre-releases/0.5.0-rc.2.md), 2026-05-19
- [`0.5.0-rc.1`](changelogs/pre-releases/0.5.0-rc.1.md), 2026-05-19
- [`0.3.0-rc.3`](changelogs/pre-releases/0.3.0-rc.3.md), 2026-05-19
- [`0.3.0-rc.2`](changelogs/pre-releases/0.3.0-rc.2.md), 2026-05-19
- [`0.3.0-rc.1`](changelogs/pre-releases/0.3.0-rc.1.md), 2026-05-19
- [`0.2.0-rc.1`](changelogs/pre-releases/0.2.0-rc.1.md), 2026-05-18
