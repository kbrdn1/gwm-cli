# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/), one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **`W` opens the Working Tree listing at full size**
  ([#592](https://github.com/kbrdn1/gwm-cli/issues/592)). The sidebar's
  Working Tree pane is one block among five in a column that is a fraction of
  the screen, so a worktree with more than a handful of changed files could
  only be read two rows at a time through `J` / `K`. `W` now opens the same
  file-explorer tree as a full-size overlay: same icons, same per-category
  colours, the same change counts on the bottom rule, scrolled with
  `j` / `k`, `g` / `G`, closed with `Esc` / `q` (or `W` again, whatever `W`
  gets rebound to, see #613 below), and rebindable under
  `[tui.keys.modal.working_tree]`.

  The listing is read when the overlay opens rather than taken from the
  sidebar's cache, so it does not go blank in the two states where that cache
  is never built: sidebar hidden, or the Details panel showing stashes. The
  read runs on a worker and the overlay opens on a loader, so a repository
  whose untracked walk is slow does not freeze the event loop on the
  keypress.

  The right of each row says how many lines the file gained and lost
  (`+120 -34`), from one `git diff` against `HEAD` in the same read, so
  staged and unstaged changes are counted together. A directory, an untracked
  file and a binary file carry no counts: the first has no diff of its own,
  and for the other two git counts no lines. The column rides its own rect on
  the right and is dropped whole on a terminal too narrow to keep it and a
  readable file name, so the name is never what goes. `D` / `U` page the
  listing by half a screen, and the key is advertised in both pane footers,
  matching the commit listing (#593).
- **`o` on the agents overlay resumes the session in the multiplexer**
  ([#591](https://github.com/kbrdn1/gwm-cli/issues/591)). The overlay told you
  which agent was working where and then left you to get there by hand. `a`
  did not help: it is a pin, it changes gwm's bookkeeping, not where the
  session runs. `o` opens a pane running the selected session.

  **In the worktree the overlay is about**, not in the session's recorded
  directory. A pinned session is pinned precisely because that directory names
  the wrong tree, and for a pinned Claude session it can be the slug directory
  under `~/.claude/projects` rather than a worktree at all.

  **Multiplexer only**, deliberately. With none active the key says so and
  does nothing, because the point is to put the session next to gwm and the
  PTY overlay would cover gwm instead. It opens at the level `mux_open_in`
  names, exactly as `t` does. One target stays refused: a zellij **tab** takes
  no trailing command in any form.

  **herdr works too, in two steps.** None of its levels accepts a trailing
  command, so gwm opens the container, waits for its new shell to reach a
  prompt, then types the line in through the pane id herdr's response carries.
  All three of `pane split`, `tab create` and `workspace create` name a pane
  to run in. The wait is load-bearing rather than defensive: `herdr pane run`
  types into the interactive shell instead of exec'ing, so a line sent while
  the shell is still running its rc files lands in the middle of that output
  and is dropped, measured on a worktree with `direnv` and a nix flake where
  it took about a minute to settle. The whole sequence therefore runs off the
  event loop, the status bar reads `opening agent pane…` meanwhile, and it
  gives up after two minutes rather than leave a worker running.

  What the pane runs is `[tui.agent_resume]`, defaulting to
  `claude -r {session}`, `codex resume {session}`, `opencode -s {session}` and
  `vibe --resume {session}`, measured against the installed binaries. They are
  configuration rather than a hardcoded table because they are four
  third-party CLIs on their own release cadence. The session id is read out of
  each tool's own artefacts, so it reaches the shell quoted through a
  single-pass expander, the same rule the hook placeholders learned in
  GHSA-fffq-vg6f-gxqm.

  A session that has ended resumes without comment; a live one is flagged on
  the status bar, since resuming it in a second pane while it runs elsewhere
  may fork or refuse depending on the tool.

- **`c` opens the commit listing full size, with load-more**
  ([#593](https://github.com/kbrdn1/gwm-cli/issues/593)). The sidebar's
  Commits pane is a fraction of a sidebar shared with four other blocks, and
  it stops at 300 commits: seeing further meant leaving gwm for lazygit. `c`
  now paints the same graph on the whole canvas, and `m` re-reads one page
  deeper, up to 1500 commits, so history is paged rather than capped. The
  title carries the row count and a trailing `+` while a deeper page exists;
  the `load more` hint disappears once the revwalk runs out of history or the
  cap is reached, so the key is never advertised where it would do nothing.
  The walk runs on a worker, never on the keypress: it sorts
  `TIME | TOPOLOGICAL`, so it traverses the whole reachable graph before it
  yields a row and the limit bounds the output, not the latency. The overlay
  opens on a loader and fills in when the read lands.
  The rows are snapshotted at open rather than read from the sidebar cache,
  which is only rebuilt while the sidebar is open and in `commits` mode, so
  the overlay works with the sidebar hidden or showing stashes. Scroll is
  `j`/`k`, `g`/`G`, all rebindable under `[tui.keys.modal.commits]`.

  Each row carries, on its right, what the hash / initials / subject columns
  do not say: the author, what the commit changed (`3~ 1+ 2- +120 -34`, in
  the Working Tree pane's colours, empty categories omitted) and how long
  ago it landed. Three tiers, picked on what the **subject** can spare
  rather than on the terminal width, since the graph is as wide as the
  branch topology makes it: `author · counts · age`, `counts · age`, the age
  alone, nothing.

  The counts arrive from a second, chained read, so the log is on screen
  immediately and the column grows about a second later (up to three on the
  deepest page). One `git log --raw --numstat` over the rows already shown
  costs about a second where a `diff_tree_to_tree` per commit costs
  thirty-three, measured. `--diff-merges=first-parent` is load-bearing:
  without it `git log` says nothing at all about a merge, and this project
  merges rather than squashes.

  `D` / `U` scroll half a screen, matching the rich PR view.

- **Two settings for what a mux spawn opens, and where**
  ([#589](https://github.com/kbrdn1/gwm-cli/issues/589),
  [#608](https://github.com/kbrdn1/gwm-cli/issues/608),
  [#611](https://github.com/kbrdn1/gwm-cli/issues/611)). The TUI's `t` key
  took whatever each backend felt like giving it. Two `[tui]` keys now say:

  ```toml
  [tui]
  mux_open_in        = "pane"    # "pane" | "tab" | "workspace"
  mux_pane_direction = "right"   # "right" | "down" | "left" | "up", pane only
  ```

  `"tab"` is a whole screen of its own: a tmux window, a zellij or herdr tab,
  one thing under three names. `"workspace"` is herdr's level above a tab and
  runs `herdr workspace create --label <name> --cwd <path> --focus`.
  `mux_pane_direction` is also the direction a bare
  `gwm tmux|zellij|herdr <pattern> --split` takes, and the new
  `--direction <dir>` overrides it for one invocation. Both keys cycle
  live in the Settings panel under the **TUI** tab.

  `mux_pane_direction` takes all four compass points
  ([#611](https://github.com/kbrdn1/gwm-cli/issues/611)). `left` and `up` are
  tmux's `-h -b` / `-v -b` (`-b` flips the side on the axis `-h` / `-v`
  picked, measured on 3.7c through `split-window -P -F`) and zellij's own
  words. **herdr takes only `right` and `down`**, declaring `[possible values:
  right, down]`, so the other two are refused there rather than substituted:

  ```
  herdr splits only right or down: left and up are tmux and zellij directions
  ```

  **`"workspace"` is refused on tmux and zellij, not downgraded to a tab.**
  Neither has a level there, and quietly opening something else would leave
  the setting describing what did not happen. The status bar names the backend
  that cannot and the one that can. (Both have *sessions*, the structural
  analogue, but gwm runs inside one: tmux would need two commands to create
  and switch to a sibling, and zellij refuses to nest sessions.)

  One caveat, the same shape as the herdr one below: a `[tui.macro*]` with
  `open_in = "mux_pane"` falls back to the PTY overlay under `"tab"` on
  zellij and under `"workspace"` on every backend, because those verbs take
  no trailing command to run. The status bar names which one refused.

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

- **Modals follow `[tui] layout` instead of always being bordered**
  ([#594](https://github.com/kbrdn1/gwm-cli/issues/594)). `compact` has been
  the default layout since
  [#545](https://github.com/kbrdn1/gwm-cli/issues/545), and every surface
  honoured it but the overlays, which kept their rounded box whatever the
  config said. They now spend the same chrome the panes do:

  - the title rides a **filled band on the frame's first row**, the same
    band a compact pane's header wears, mixed from the modal's own role so
    a delete or a merge confirmation keeps its danger colour and the two
    worktree forms keep their green;
  - **no rules on any side**, top or bottom included;
  - the row every modal already spends on its key hints is painted as a
    quiet `section_bg` **footer band**. It is a ground under a row that was
    already there, not an extra one, so nothing moved to make room for it.
    A bordered modal's bottom-rule counter (the Working Tree's per-category
    counts) rides the right of that band;
  - **a blank row at each end of the content**, so nothing sits flush
    against a band. The boxed layout's interior padding already gave that,
    and the four full-size overlays plus the note editor gained the one
    above their hints under both layouts.

  That is **two rows and four columns back per overlay**, which is what the
  layout was asked for in the first place: modals are the surfaces most
  likely to overflow a short terminal.

  A rule around a panel floating over content is worth something, and what
  replaces it is the ground: while a compact modal is up, everything behind
  it is **darkened**. The colours are mixed toward black rather than only
  dimmed, because `DIM` reaches the foreground alone and a pane's header
  band sits directly above a full-size overlay's own band. A palette with no
  components to mix, an ANSI colour name or a 256-palette index, keeps `DIM`
  by itself.

  `layout = "bordered"` is the opt-out and is untouched, rules, padding,
  sizes and undimmed background alike.

- **`c` and `C` now mean the same thing in both panes, which moved three
  bindings** ([#593](https://github.com/kbrdn1/gwm-cli/issues/593)).
  `c` opens the commit listing and `C` the CI checks, in the worktrees pane
  and in the status pane alike. A key that changes meaning under the focus
  is a key you have to think about, so:

  | Action | Was | Now |
  |:---|:---|:---|
  | `commits` | (new) | `c` |
  | `ci_checks` | `C`, plus a contextual `c` on the status pane | `C` everywhere |
  | `edit_worktree` (rename) | `c` | `e` |
  | `exit_to_worktree` | `e` | `E` |

  The contextual routing from
  [#436](https://github.com/kbrdn1/gwm-cli/issues/436), which existed to give
  the status pane its own `c` for the checks, is gone with it, and the PR
  line's CI badge no longer changes between `[c]` and `[C]` under the focus.
  Existing `[tui.keys]` overrides are untouched; only the defaults moved.

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

- **An overlay's toggle key closes it whatever it is bound to**
  ([#613](https://github.com/kbrdn1/gwm-cli/issues/613)). `3`, `4` and `W`
  each close the overlay they open, but the guard doing it asked
  `key_matches_action`, which reads a single stroke and only ran after the
  modal verbs had their turn. Two silent holes: a multi-stroke binding
  (`working_tree = ["g w"]`) could open the overlay and never shut it, and a
  binding the overlay's own context already claimed (`= ["j"]`) opened it and
  then scrolled it. The toggle now resolves first, against that one action
  rather than the whole keymap, and it accumulates its chord, so a prefix
  stroke is consumed instead of firing a scroll verb on the way through.

  Each of the three overlays routes its keys through an `App` method now
  (the shape the create overlay has had since #217), because the ordering
  is the fix and a `match` in the run loop cannot be tested. `d` still
  cannot reach the delete confirm from behind an overlay: the toggle
  resolves against its one action, not the whole keymap.
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
