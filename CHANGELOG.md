# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/), one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **herdr is a third multiplexer backend**
  ([#588](https://github.com/kbrdn1/gwm-cli/issues/588)). `gwm herdr <pattern>`
  opens the matched worktree in a new [herdr](https://herdr.dev) tab, `-p`
  splits the current pane instead, and the TUI's `t` key finds herdr the way
  it finds tmux and zellij. Detection reads `$HERDR_ENV`, which herdr sets in
  every pane it manages, and it comes last in the cascade so nothing changes
  for a tmux or zellij user.

  Under the hood: `herdr tab create --workspace <id> --label <name> --cwd
  <path> --focus` and `herdr pane split --current --direction right --cwd
  <path> --focus`, verified against a live herdr 0.8.2 rather than its help
  text. The split needs a direction because herdr's parser has no default for
  one, and `right` is the analogue of tmux's `-h`. The other two flags are
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

- **Merge a PR from the TUI** ([#551](https://github.com/kbrdn1/gwm-cli/issues/551)).
  `m` from the worktree table merges the selected row's linked PR; `m` inside
  the PR / issue view merges the active tab's. Both go through the delete
  flow's confirmation, and it is the same modal: same layout, same countdown,
  same spinner while it runs, same buttons hidden mid-flight. Its summary
  names the PR, `head → base`, the resolved method and what it does to the
  history, and the CI rollup. `m` cycles the method from inside it, and a
  failure keeps the modal up with the forge's own message.

  The check state is shown rather than enforced: a forge refuses a merge for
  reasons gwm does not model, and its own error says which. **The source
  branch is never deleted**: neither backend is ever asked to.

  The method comes from the new `merge_method` key and defaults to `merge`,
  the least destructive of the three:

  ```toml
  merge_method = "merge"   # or "squash", "rebase"
  ```

### Changed

- **The rich PR / issue view (`I`) had its design pass** ([#551](https://github.com/kbrdn1/gwm-cli/issues/551)). It was
  built to get the data on screen and had never been laid out; the compact
  layout of 1.8 made its own density the next thing that read as unpolished.
  Six things changed:
  - **The issue and the PR are two tabs**, switched with `Tab`. The view
    still opens on the PR, which left the issue unreachable from a worktree
    in review. A PR landing while the view is open still replaces an issue
    that was only standing in for it, and does not replace one you tabbed
    to.
  - **Bodies render as Markdown** rather than as their source. Headings,
    emphasis, inline code, fenced blocks, lists, task lists, block quotes,
    GitHub alerts, links by their text, and HTML comments not shown at all.
  - **Nothing is capped.** The view scrolls, so the window is the terminal
    and the row count costs only the rows. Descriptions, reviews and the
    whole conversation render in full; a `… N more` row now only reports
    what the fetch itself did not return.
  - **The metadata block wears the Status pane's colours**, resolved through
    the pane's own helpers rather than a second set of rules.
  - **A width policy of its own**, 80% of the terminal capped at 120 columns
    against the shared overlay's 62% capped at 88. That ceiling was chosen
    for the clean report, whose rows stop earning columns; prose does not.
    A label-less row also spans the whole inner width now, which the view
    was paying for twice.
  - **Code and diff lines are kept whole and scroll sideways** with `h` /
    `l`. In YAML or Python the indentation is the program, and a wrapped
    `+` line's continuation carries no sigil and reads as context.
  - **`y` copies the active tab's URL, `Y` its description.**
  - **Pager motions**: `D` / `U` move half a window, `g` / `G` jump to the
    ends, and `c` opens the same PR's CI checks without leaving the view.
  - **A modal opened from the view closes back to it**, on the tab that was
    being read. The CI list and the merge confirmation are both reached from
    inside it, and landing on the worktree table meant re-selecting the row
    and pressing `I` again to carry on reading. Opened from the table, both
    still close to the table.

### Fixed

- **A compact pane's header says where you are, and its name no longer
  dims when it is not**
  ([#605](https://github.com/kbrdn1/gwm-cli/issues/605)). In the default
  compact layout the header carried the focus signal twice, and both halves
  were weak. The text was repainted from `focus` to `muted` — so a pane's
  name, the thing you read to know which pane to `Tab` into, was rendered in
  the role reserved for deliberately secondary text the moment it went
  inactive, while the spans that already carry a colour (the filter `/`
  prompt, the Working Tree counts) did not follow, leaving one header line
  running two rules side by side. And the fill under it stepped from
  `section_bg` to `selection_bg`, two tones that are adjacent by design (14
  grey levels apart on `claude-dark`) and that read as a permutation of grey
  rather than as a place.

  The two states now trade the same pair of roles instead of dimming one of
  them. An inactive header is `accent` text on the `section_bg` band; the
  focused one is that band's tone written on an **`accent` band**, bold —
  the same dark-on-colour treatment the version chip and the footer's
  context anchor already use. `muted` appears in neither, the focused pane
  is findable without hunting, and the header no longer borrows
  `selection_bg` from the cursor row.

  The band is `accent` pulled down toward `section_bg` rather than `accent`
  at full strength, which was too loud, and rather than `focus` — the border
  tone, which is *more* saturated and so does not fix the half of "too
  strong" that darkening does. It is mixed from the two roles it sits
  between rather than declared as a sixth background role, so a `[theme]`
  override of either keeps them in tune; a palette with nothing to mix — an
  ANSI name, whose value belongs to the terminal, or a 256-palette index,
  which is the default theme's case — keeps `accent` itself rather than
  falling back to a grey. How far it can be pulled down is bounded by the
  dark text written on it: the two keep the 3:1 WCAG asks of bold display
  text, which is pinned by a test.

  Spans that carry their own colour keep it on either band — the header
  style is patched onto them, not substituted — so a filter prompt or a
  per-category count still says what it says. The right-flushed counter
  follows the title, which bordered mode already does with the border
  colour. An inactive header is no longer bold, which is what makes the
  weight a signal.

  Bordered mode is otherwise untouched: there the accent still paints the
  four rules and the title inside the top one.

- **A linked row with nothing fetched yet is white, not green and purple**
  ([#596](https://github.com/kbrdn1/gwm-cli/issues/596)). The table's `I/P`
  marker painted its two placeholder slots with a different status role each:
  `clean` green for the issue, `locked` purple for the PR. So one row said two
  different things about the same missing data, and both colours were on loan
  from a loaded state (`clean` is an open issue and an open PR, `locked` is a
  merged PR, a closed issue, and the locked-worktree badge). That is the state
  every linked row launches in, since nothing is fetched until `F`. Both slots
  now take `name`, the one role in the marker that neither badge map can
  produce and the colour the empty slot beside them already uses. The glyph
  still tells the two apart: `-` is "no link", `●` is "linked, not fetched
  yet".

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
