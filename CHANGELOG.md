# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/), one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **`gwm doctor` reports the config a deleted branch left behind, and
  `--fix` drops it**
  ([#633](https://github.com/kbrdn1/gwm-cli/issues/633)). `gwm create`
  writes about seven `branch.<name>.gwm-*` keys per branch and nothing
  removes them when the branch goes away, so `.git/config` only grows. That
  is not cosmetic: libgit2 reparses the file on every `Repository::open`,
  and `statuses()` and `branch.upstream()` each take a config snapshot that
  duplicates every entry, so `gwm list` pays the file's size once per
  worktree, several times over. With 8 worktrees held constant and the
  config as the only variable, the listing goes from 45 ms at 13 lines to
  184 ms at 1434.

  The sweep matches the `branch.<name>.gwm-` **prefix** rather than a list
  of known key names, so a key added in a later release is covered without
  anyone having to remember. Keys git itself writes (`remote`, `merge`, …)
  are never touched, on a live branch or a dead one.

  The report separates keys `--fix` can drop from keys reached through
  `include.path`, which live in a file gwm does not rewrite: for those the
  hint names the only remedy that works, editing that file, instead of
  pointing at a `--fix` that has already failed on them and warning again
  every run.

  `--fix` is opt-in because it edits `.git/config`; a plain `gwm doctor`
  stays read-only. It reports what it achieved by reading the config back
  after writing, not by counting the deletions it attempted: a key pulled in
  by `include.path` lives in a file gwm does not rewrite, and libgit2
  refuses to delete it, so those are named as survivors instead of being
  claimed as purged. A purge that cannot run at all does not take the
  report down with it either: `doctor` exists to diagnose, so the checks
  still print. It works on the local config level alone, since deleting
  through the merged view would let git resolve the key up to
  `~/.gitconfig`. It leaves the empty `[branch "…"]` header behind, because
  libgit2 removes keys and not sections: on a repo grafted to 1434 lines,
  purging 869 keys brought the file to 561 lines, not to 13. That figure is
  measured, not `1434 - 869`: a multi-valued key spans several lines, and
  the keys of branches that are still alive stay put.

- **The mouse gwm was already capturing now does something, and two hidden
  panels get a place on the header**
  ([#624](https://github.com/kbrdn1/gwm-cli/issues/624)). `EnableMouseCapture`
  went out on the first frame and no `Event::Mouse` arm ever existed, which is
  worse than not supporting the mouse at all: capture takes the terminal's own
  drag-to-select away, so the cost was being paid and nothing was bought with
  it.

  Clicking now selects the worktree row under the pointer, focuses the pane it
  lands in, picks a row in any modal listing, switches a Settings tab, or
  closes a modal through the `✕` in its corner. The wheel scrolls — or moves
  the selection of — whatever is **under the pointer**, which is a deliberate
  departure from the issue's "the focused list": pointing at a pane is how a
  pointer says which pane it means. Focus stays what `Tab` and the digits set.

  The header carries `▤` and `⚙` left of the pinned version chip, opening the
  Command Logs and Settings panels the way `3` and `4` do. They are the only
  thing on screen saying those panels exist — until now the help overlay was
  the only way to find out — so they are reserved ahead of the working path in
  the row's sacrifice order.

  The geometry every click resolves against is **published by the renderer**
  as it draws, not re-derived afterwards: the layout is rebuilt from scratch
  each frame, so a click target computed from `App` state drifts the first
  time a rule changes, and the row-arithmetic rules here have changed twice
  already. A surface that is not on screen publishes no zone and therefore
  cannot be hit, which is what keeps the hit test free of any view branching.

  `M` releases the mouse outright, handing the terminal's text selection back
  until it is pressed again — and the release survives a trip through lazygit,
  an `exec` run or a review launcher, which used to re-enable capture on the
  way back. `Shift`+drag reaches the same selection without giving anything up
  on the terminals that implement it, which is most of them.

  The pointer reaches more than the issue listed, on feedback from using it:
  a sidebar section's **title** opens that section full size (`Issue / PR`,
  `Agents`, `Working Tree`, `Recent Commits` — the same modals `I`, `a`, `W`
  and `c` open), the create and rename forms focus a field on click and step
  the branch-type selector from its `‹` / `›`, and the confirmation modal's
  buttons are buttons. The `✕` and the buttons both fire by handing the event
  loop the key the user would have pressed — `Esc` and the modal keymap's
  `activate` — so a rebind reaches the mouse and no modal grows a second copy
  of its own teardown.

  `[tui] mouse` decides whether any of this is on, defaulting to `true` and
  editable live from the Settings panel's TUI tab. Reading the mouse and
  letting the terminal select text are mutually exclusive — while gwm is reading, a drag belongs to gwm — so `M` switches
  between the two for a session, `[tui] mouse = false` starts every session on
  the other side of it, and the header carries a ` mouse off · M ` chip while
  it is off — a mode with no sign on screen is one you get stuck in, with every
  click doing nothing reading as a broken build rather than as a switch you
  threw. gwm keeps
  its side of that trade as small as it can: it asks for press tracking
  (`1000`) only, never the drag and motion reporting (`1002` / `1003`) most
  TUIs turn on with it, because nothing here reads a drag. `Shift`+drag is the middle
  ground and does reach the terminal's own selection without giving the click
  up — honoured by the terminal rather than by the application, so its setting
  decides (Ghostty's `mouse-shift-capture`, iTerm2's "Terminal may report
  mouse events").

  The PTY overlay still drops mouse events, exactly as it did before: nothing
  ever forwarded them to the child, so this is the status quo rather than a
  regression. Real forwarding needs SGR re-encoding against the overlay's
  inset origin and the child's own DECSET state.

### Fixed

- **The warm-cache sidebar bench runs again, and a CI job now runs the
  benches** ([#634](https://github.com/kbrdn1/gwm-cli/issues/634)).
  `benches/sidebar_cache_hit.rs` drew one frame and asserted the sidebar
  cache had filled itself. That was the contract when it was written: the
  first frame ran `git log` and stored the rendered sections. #351 took git
  off the render path, so a frame warms nothing, and the bench has panicked
  on every run since, 1086 commits ago. `cargo bench` stops at the first
  failure, so it also masked the third bench, which was healthy all along.

  Nobody saw it because nothing ran the benches. Both halves are fixed
  here: the bench seeds the payload the way the event loop does, and a
  `bench` job runs all three on every push and pull request. The job passes
  criterion's `--test`, which runs each benchmark once and measures
  nothing, so it fails on a panic or a build break and never on timing
  noise from a shared runner. A perf gate that goes red on a slow runner is
  a perf gate somebody switches off.

  The old assertion does not come back. `cache.is_some()` is what let this
  rot: the renderer serves the cache only when its key matches the current
  selection and mode, so a payload under any other key renders the loading
  placeholder and the bench would still report a plausible number for
  drawing it. The timed frame is asserted to carry a real commit subject
  instead.

- **A config key with no value no longer crashes gwm**
  ([#633](https://github.com/kbrdn1/gwm-cli/issues/633)). A git config entry
  may carry no value at all (`\tgwm-agent-pin` with no `=`, git's
  implicit-boolean form). `Config::remove_multivar` runs its value regex
  against such an entry and segfaults inside libgit2, taking the
  `.git/config.lock` it had already opened with it, after which every config
  write in that repo fails, git's own included, until the lock is deleted by
  hand. `gwm agents detach` died that way, and so did the new
  `gwm doctor --fix` before this.

  The class is not "deleting", it is **every libgit2 config call that takes
  a value regex**: `remove_multivar` and equally `set_multivar`, whose
  pattern selects the value to replace. Stating it too narrowly is what let
  the crash survive two rounds of fixing, so the call sites are now
  enumerated by a test rather than by memory: adding one fails the suite
  until it is guarded. Reading is separate and simpler, `ConfigEntry::value`
  panicking on the same entry by documented contract. `gwm agents` reads past a
  valueless pin, `detach` clears what it can, and the one shape libgit2
  offers no safe primitive for (a key both multi-valued and partly
  valueless) is refused with a message naming the line to delete, rather
  than crashed on or half-deleted.

### Changed

- **Two TUI views move to `src/tui/views/`, one module each for their
  state, their rendering and their keys**
  ([#635](https://github.com/kbrdn1/gwm-cli/issues/635)). `state/` was
  already split one module per view, but the behaviour lived in `app.rs`
  (9765 lines) and the rendering in `ui.rs` (10366), so adding or changing
  a view meant editing three files, two of them enormous. The layering was
  horizontal while the variability is vertical. `views/commits.rs` and
  `views/exec_picker.rs` are the pilot.

  No behaviour change. The view move itself is invisible from outside the
  crate — `views` is private and every `gwm::tui::*` path resolves through
  the same re-exports — and it edits no test at all. The satellite
  repatriation that follows does touch five test files, but only to follow
  a field path: `app.edit_failure` became
  `app.create_form.edit_failure`. Undoing each rename with sed reproduces
  every one of those files byte for byte, so no value, assertion, case or
  test name moved. Two paths that did go away are
  `gwm::tui::state::commits` and `gwm::tui::state::exec_picker`, whose
  modules moved to `views`; everything they held is still re-exported at
  `gwm::tui::*`, and `src/lib.rs` declares the library an internal test
  seam with no SemVer guarantee (#342).

  The Commits overlay's key routing was a `match` sitting inside the run
  loop, which is the one shape a test cannot reach — the hole
  [#613](https://github.com/kbrdn1/gwm-cli/issues/613) named. It is now
  `App::handle_commits_key`, and four tests pin what it does, including
  the thing that distinguishes it from the Working Tree overlay: Commits
  resolves modal verbs only, with no global-toggle block in front, so a
  global `commits` key rebound onto `j` still scrolls here.

  **`App` goes from 85 fields to 60.** The issue's second scope point asks
  for the orphaned satellite fields to come home, on the two moved views
  and on any modal whose extraction stopped halfway, and all of them do:
  `ExecPicker` takes its captured config, commondir and container counter;
  `CleanOverlay` its config and countdown; `DetailOverlay` the worktree and
  forge link it was built for; `CreateForm` the two failure banners and the
  Edit origin; and `ConfirmContext` what the confirmation is actually
  confirming. Three modals had no state module at all, so `state/help.rs`,
  `state/agents.rs` and `RichView` in `state/rich_view.rs` are new.

  Three of those moves put a field next to a method that resets its
  neighbours, and each of the three would have been a silent behaviour
  change: `ExecPicker::container_seq` is the monotonic half of a
  containerised run's `--name`, so resetting it in `open` makes two runs on
  one worktree collide; `DetailOverlay::target` / `link` are pinned by the
  consumer *before* `open`, so clearing them there leaves the agents
  overlay attaching against nothing; and `CreateForm::reset` must leave the
  failure banners up, because one caller resets the form without clearing
  them. All three are now tests, each proven red by making the mistake.

  **The churn the pilot was meant to measure did not move, and the reason
  is worth writing down.** Of 2251 commits, 512 touch `app.rs` or `ui.rs`;
  21 of those touch either of the two moved surfaces, about 4%. Measured
  per view over the same 512, the ranking is roughly the inverse of the
  "most independent first" order the issue proposed: `sidebar` 61,
  `create_form` 41, header/footer 36, `detail_overlay` 33,
  `working_tree` 32, `config_panel` 27, against `commits` 13 and
  `exec_picker` 9.

  Those integers are upper bounds, not clean counts, and the method is
  why. They come from `git log -G` over identifiers named after each view,
  restricted to those two files, so a commit that only touched
  `app.commits.scroll` in the run loop counts for the Commits view even
  though that line is not moving. They over-count rather than under-count,
  which cuts the same direction as the conclusion. `git log -L
  :funcname:` would have been exact and works on `ui.rs`, but git's Rust
  funcname pattern does not match a method indented inside an `impl`, so
  it returns zero for every method in `app.rs` — a silent zero, not an
  error. The ordering is what carries the argument, and the ordering
  survives the imprecision.

  The views that are cheap to extract are cheap precisely because nothing
  changes them. So the mechanism works and the two pilot files are the
  right shape, but continuing down the independence list buys nothing:
  the next extraction worth doing is `sidebar` or `create_form`, and
  those are coupled, which is a different and larger piece of work than
  this one. Not folded in here.

- **CI runs the test suite under `cargo-nextest`**
  ([#634](https://github.com/kbrdn1/gwm-cli/issues/634)). `cargo test` runs
  110 test binaries in sequence, each with its own thread pool.
  `cargo-nextest` schedules all 3525 tests across one global pool and gives
  each test its own process. The tests themselves are untouched and all 3525
  pass under it.

  The speed case is weaker than the issue expected, which is worth writing
  down rather than leaving the next reader to assume otherwise. Measured
  over 11 `cargo test` runs on `dev` against 4 of the swapped job,
  normalised per test because the suite grew from 3273 to 3525 tests across
  that window, the median moves -8.9% on ubuntu, -6.5% on macos and +0.8%
  on windows. Real and in the same direction on the two Unix runners, but
  single-digit, and inside a far wider band: 29.2s to 42.2s of execution on
  ubuntu for the same command on the same suite. Every macos and windows
  sample falls inside the `cargo test` spread. The pooling gain the issue
  measured came off a local 8-core machine; a runner has fewer cores, so a
  per-binary pool already saturates them and process-spawn overhead eats
  what is left. The test step is compile-bound either way.

  What the swap does buy is process-per-test isolation. It surfaced a
  shared-fixture race in the test harness on its first run: the git shim
  directory is a fixed path, and the `OnceLock` guarding it only serialises
  the tests sharing one process, so process-per-test had several of them
  race an unlink-then-symlink. The link is staged and renamed into place
  now, which is atomic.

  Doctests are the one thing nextest gives up, so `cargo test --doc` runs
  next to it. On ubuntu only: a doctest behaves the same on all three
  runners, and that job carries minutes of slack against windows in the same
  matrix, so the roughly 35 seconds it costs never reaches the workflow's
  critical path. The MSRV job still compiles at the declared floor and runs
  no tests.

- **`gwm list` scans its worktrees in parallel**
  ([#633](https://github.com/kbrdn1/gwm-cli/issues/633)). Every row opens
  its own `Repository`, runs `statuses()` over that worktree and walks its
  branch age; nothing in that body touches the main repo, and it is where
  a listing's time goes. It now runs on up to `available_parallelism()`
  workers, work-stealing off an atomic cursor, the same shape
  `gwm exec` has used for its fan-out since #313.

  Row order is unchanged and guarded: `repo.worktrees()` is not sorted and
  the table renders what it returns, so results are written into per-index
  slots rather than collected as they complete. Everything read off the
  main repo stays on the calling thread, `&Repository` not being `Sync`.

  Measured with hyperfine, mean of 20, 8 worktrees, config size the only
  variable: 83 ms to 51 ms at 224 lines, 98 ms to 73 ms at 538, 184 ms to
  118 ms at 1434. Combined with a `gwm doctor --fix` on the same repo,
  184 ms becomes 100 ms. That last figure does not interpolate from the
  table: purging leaves 193 empty `[branch "…"]` headers behind, so a
  561-line config that has been swept is not the same object as a 538-line
  one that never grew.

  This also speeds up `gwm statusline`, the daemon and every TUI refresh,
  which share the same listing path.

- **The Settings panel gets a value column, named sections and tab glyphs**
  ([#623](https://github.com/kbrdn1/gwm-cli/issues/623)). The panel carries the
  most content of any modal and showed it the most plainly: values were a span
  glued after a fixed 24-cell label pad, so a tab of sixteen rows had sixteen
  different value positions, and the one 26-character label overflowed the pad
  and pushed its value two cells right of every other. The `TUI` tab was a
  single undivided run mixing layout, sidebar, editing, open, multiplexer,
  browser and refresh knobs, and the five tabs were bare words.

  Every tab now reads as two columns: what a row is on the left, what it is set
  to on the right, right-aligned against the panel's own edge so its width is
  used rather than left empty beside a content-sized block. On the editable
  tabs the shape carries the kind: a choice reads `‹ value ›`, a boolean reads
  `[✓]` / `[ ]`, a typed value reads plain, and an optional text field nobody
  has set reads `(unset)` instead of leaving the column blank. `Keys` puts its
  binding there and `All` its resolved value, dropping the ` = ` that used to
  join a key to it. On a panel too narrow for both columns the gap falls back
  to a minimum and the row pans sideways, as it always did.

  A section rule mutes its separator and not its name: the rule is chrome and
  recedes, while the name keeps the theme role the bare heading wore before it
  was given a rule to sit in.

  The bootstrap report's `Logs` becomes a subtitle. It was a nested section
  with its own chrome, which under `[tui] layout = "compact"` paints the same
  full-width accent band the modal's own title rides, so the two stacked and
  the second read as a second title.

  The Keybindings overlay (`?`) gets the same treatment, which is the pass
  #623 deferred until the Settings panel had proved it out. Its description
  leads and its chord is the right-hand column, the order the Settings Keys
  tab already used for the same data; its `Blank / Section / Blank` headings
  become one blank and one labelled rule, giving back a row per section on a
  body with a dozen of them, with the single blank kept so a section does not
  read as a continuation of the one above it; and it is a little wider, because
  60% of a 100-column terminal is 64 cells for a body that now wants about 73.

  Forty of its descriptions were shortened to fit beside their chords, and one
  fixed chord string with them: `Left/Right/Up/Down` was eighteen cells, and
  being the widest in the body it set the key column for every row. At 80
  columns nothing is truncated now, pinned by a test, so a new binding with a
  long description fails rather than quietly shortening every other row's.

  Sections are labelled rules across every tab: the `TUI` tab gains seven
  (Appearance, Sidebar, Editing, Open, Multiplexer, Browser, Timing), which
  reorders its fields into those runs, and the `[global]` / `[table]` headings
  the `Keys` and `All` tabs already had become the same rule. A blank sits
  ahead of every break and under the tab strip, so a section does not read as a
  continuation of the one above it and the first rule opens the body rather
  than hanging off the chrome. Each tab is led by a codicon glyph.

  That spacing costs rows, so [#569](https://github.com/kbrdn1/gwm-cli/issues/569)'s
  height bounds move from `(11, 32)` to `(12, 40)`: the `TUI` tab is 29 body
  rows now, and at the old ceiling a form the user opened deliberately would
  have scrolled by seven. `modal_height` still takes `term_height - 4` after
  the ceiling, so a short terminal is unaffected and scrolls as it did.

  `←` / `→` now adjust the selected choice, one value back or forward. The
  issue described them as already doing this; they did not, since
  `ConfigScrollLeft` / `ConfigScrollRight` were gated to the `All` tab's
  horizontal pan and were dead everywhere else. The `‹ ›` markers are what
  advertises them. No binding changed.

  The footer gains a `j/k move` verb, and `←/→ adjust` replaces `Space cycle`
  on a cyclable field rather than joining it: `modal_hint_line` centres its
  spans, so naming one operation twice overflowed the line and clipped it at
  both ends, costing the leading hint outright and leaving `Esc close` as
  `Esc c`. Five verbs still need nine more columns than the four that came
  before, so on a panel too narrow for them `move` drops out instead: the
  footer keeps fitting every terminal the old one fitted, rather than trading
  a narrow terminal's `Esc close` for a movement hint every TUI shares.

- **The doc captures regenerate as one step, in the order the traps require**
  ([#631](https://github.com/kbrdn1/gwm-cli/issues/631)). Regenerating the set
  was a four-step sequence held together by a maintainer's notes: bump,
  `cargo install`, `generate.sh` for 22 of the 24 tapes, then `demo.tape` and
  `github-linking.tape` by hand. Every step had an ordering constraint that was
  invisible until it bit, and none of it was visible to CI: the files exist,
  the widths pass, `vhs` exits 0.

  `docs/_capture/generate.sh` now covers the whole set and owns the order.
  It builds the `gwm` it drives from the tree being captured and puts it first
  on `PATH`, instead of documenting whichever build a shell happens to resolve;
  v1.10.0 came within a commit of publishing captures 175 commits stale that
  way, correctly sized and green. `github-linking.tape` runs first, before
  anything else writes under `docs/`, and only when the repo's main checkout is
  clean and its branch has an open PR, so the release commit cannot leak into
  the Working Tree pane it photographs. The main checkout, not the current
  directory: the pane follows the selected row, which is row 1 wherever the
  tape ran, and the tape now takes that path from the script rather than
  hardcoding one. `demo.tape` runs last, because it is the one tape that
  changes the fixture for the others. Both were previously outside the loop and
  unreported, so a run finished on a tick over two assets it had left stale.

- **The Working Tree listing reads as columns**
  ([#622](https://github.com/kbrdn1/gwm-cli/issues/622)). The `M` / `A` / `D`
  / `?` status letter used to lead the file name, so on a 27-file change set
  it sat at a different offset on every row and the eye had to re-find it. It
  is now a right-aligned column of its own, in both the sidebar pane and the
  full-size overlay, and it keeps the per-category colour it always had.

  The column is pinned to the right edge of its surface at every width, and
  the letter is priced far below what the `+N -M` column costs: a pane or an
  overlay too narrow to seat the counts still seats the letter. Before this
  the letter was an inline badge two cells wide that no width ever dropped,
  so charging it the counts' floor would have lost a capability rather than
  yielded a column.

  The `+N -M` line counts from #592 now ride the sidebar pane too, inside
  the letters. That pane had deliberately stopped at the rows to save the
  `git diff --numstat` they need, since it re-reads on every selection
  change; it pays for that read now, on the sidebar worker rather than the
  render path.

  In both surfaces the letter shares the right end of the row with those counts,
  and the two yield in a fixed order: the letter is carved out first, so a
  narrowing terminal drops the counts and never the letter. `+N -M` says how
  much a file changed, the letter says what happened to it, and a row that no
  longer says what it is has lost its subject rather than a detail.

  The overlay title carries the changed-file count, `Working Tree (27)`. It
  comes from the per-category counts rather than the row count, so it agrees
  with the footer: the rows also hold directories, the `… N more` overflow
  notice and the `✓ clean` sentinel, none of which is a changed file. It is
  withheld while the listing is still being read, since `(0)` there would be a
  claim rather than a count.

  Directory rows lead with a `▾` disclosure caret instead of the folder glyph
  they carried since #300. A folder glyph and a file glyph are two icons of
  the same weight, which left indentation alone to separate the two levels;
  a caret is a different shape. File rows keep their per-extension glyph.

  The collapsed leading path the issue also asks for was already there:
  `build_tree` has folded single-child directory chains into one `a/b/c` row
  since #300.

### Added

- **A guard on the version the captures advertise**
  ([#631](https://github.com/kbrdn1/gwm-cli/issues/631)). 17 of the 24 tapes
  open the TUI, whose header paints a `gwm X.Y.Z` chip, so a set regenerated
  before the version bump advertises the previous release for the life of this
  one. v1.8.0 shipped that way and v1.10.0 repeated it.

  Reading the chip back out of the pixels would need OCR. `version-stamp.tape`
  instead asks `gwm --version` **through vhs**, from the same shell and the
  same `PATH` every other tape resolves `gwm` through, `generate.sh` aborts the
  run when the answer is not the version in `Cargo.toml`, and commits it to
  `docs/_capture/captured-version.txt` once the run completes. The new
  `docs_assets_tests::captures_were_generated_at_the_manifest_version` compares
  the two, so the release PR goes red rather than the docs going stale, and the
  tag cannot be cut from a set that documents another version. The tape reports
  the path as well as the version, and the run refuses to continue unless it is
  the file cargo just built: a stale binary carrying the same version number
  answers a version check perfectly, which is the shape the v1.10.0 near miss
  had. `tests/capture_pipeline_tests.rs` drives all of that against a throwaway
  repo with stubbed tools, and a companion guard pins the phase order rather
  than leaving it to the prose.

### Docs

- CONTRIBUTING.md § Releases carries the capture step and the three ordering
  constraints behind it, and `docs/_capture/README.md` no longer describes two
  tapes as living outside the script
  ([#631](https://github.com/kbrdn1/gwm-cli/issues/631)).

- The five `:::` callouts under `docs/` are GFM alerts
  ([#641](https://github.com/kbrdn1/gwm-cli/issues/641)). Container syntax is
  markdown-it / Nuxt Content, and this tree is read by neither: GitHub emitted
  the directive as literal text in a paragraph, and Starlight wants
  `:::caution[title]` with the colons glued to one of four names it knows,
  which `::: warning` is not. So the security callout on the GitLab page, the
  one explaining that a bare `forge` key authorises nothing, was raw
  punctuation in both places. The one that carried a title keeps it as a bold
  first line inside the quote, since a GFM alert has no title slot.
  `docs/fr/5.integrations/1.github-linking.md` gains the multi-forge callout
  its English half already had. `tests/docs_callout_tests.rs` rejects the whole
  `:::` prefix rather than the spaced form the tree happened to carry:
  `:::note` is valid Starlight and would render on the site while staying
  literal on GitHub, which is the same defect seen from the other side. Astro
  does not implement GFM alerts natively, so the matching half, folding them
  back into Starlight asides, lives in
  [kbrdn1/kbrdn-docs#81](https://github.com/kbrdn1/kbrdn-docs/issues/81).


## Past releases

In reverse chronological order:

- [`1.10.0`](changelogs/1.10.0.md), 2026-09-01
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
