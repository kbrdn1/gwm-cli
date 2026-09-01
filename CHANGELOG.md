# Changelog

All notable changes to this project will be documented here.

This file tracks the **in-progress** release only. Past releases live under
[`changelogs/`](changelogs/), one Markdown file per SemVer version.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **`Ctrl+n` opens the create form on an issue that already exists**
  ([#625](https://github.com/kbrdn1/gwm-cli/issues/625)). `gwm create --issue`
  (#617) derives the whole `<type> <issue> <desc>` triple from an issue on the
  forge, and it was CLI-only. The TUI is where a worktree usually gets created,
  so the one place already showing a worktree list and its linked issues was
  the one place that still asked for the title to be retyped as a slug and the
  branch type to be read off the labels by hand.

  `Ctrl+n` (and `create-from-issue` in the command palette) opens the form on a
  single field, the issue number. Enter looks the issue up rather than creating
  anything; when the answer lands the form becomes the ordinary structured form
  with the type, the number and the derived slug in it, and a second Enter
  creates the worktree. Prefilling rather than creating is the point: the slug
  is a guess about a title, and this is the surface that can show the guess
  before committing to it. The derivation runs through the very functions the
  CLI uses, so the two cannot produce different slugs for the same title.

  Where the CLI has to refuse, the form asks. A non-interactive command has
  nowhere to ask when the labels name no branch type or name two, which is what
  `--type` is for; the form lands the cursor on the type selector with
  everything else filled. A closed issue prefills with a warning rather than
  refusing, since nothing is written until you confirm. A number that already
  has a worktree closes the form and names it, matching the CLI's exit-0
  behaviour, and reads the same link `gwm list` shows, so a worktree attached
  by hand with `gwm link` counts too.

  The lookup runs on the async task spine, never the render path, and a result
  is applied only when the form asked for that exact number: the form is a
  second consumer of a message that also fires for the sidebar prefetch and for
  an explicit refresh. An issue already in the cache prefills straight away
  rather than waiting for a message that would never arrive, since the fetch
  coalesces on a cache hit.

  `Ctrl+t` is inert in this mode and the hint row does not offer it. The
  toggle swaps between the structured triple and the free-form name, which
  are two ways of typing the same worktree; this one is a two-step mode, left
  by answering it or by cancelling, so a third stop on the cycle would only
  make the key unpredictable.

- **`gwm create --issue <N>` opens a worktree for an issue that already
  exists** ([#617](https://github.com/kbrdn1/gwm-cli/issues/617)). `gwm new`
  covered the issue that does not exist yet: it renders the issue from
  `[issue_template.by_type.<type>]`, creates it, and opens its worktree.
  Nothing covered the other half, the issue a teammate, a bot, or you last
  week already filed, where the way through was to open the issue in a
  browser, read its title, decide which branch type its labels implied, and
  type a kebab-case slug that restated the title. Three of those four steps
  were transcription, and the transcription drifted.

  `--issue <N>` fetches the issue and derives the triple. `<desc>` comes from
  the title with the type's `title_prefix` taken back off, normalised through
  the same kebab-case path a hand-typed `<desc>` goes through and truncated
  on a word boundary rather than mid-word. The prefix is resolved through the
  same combinator `gwm new` uses when it writes the title, falling back to
  the issue form's own `title:`, so the two halves of the flow produce the
  same slug for the same title. `<type>` comes from the labels:
  `[issue_template.by_type.*].labels` is the type-to-labels map `gwm new`
  writes with, read backwards. A type declaring no labels is never a
  candidate, since an empty list says nothing about which issues belong to
  it.

  Nothing is guessed. Labels matching no type, labels matching two, and a
  repo that never configured the map are three distinct refusals, each
  naming what it saw and pointing at `--type <TYPE>`. A closed issue is
  refused too, because a worktree for one is usually a wrong number, and
  `--force` proceeds. A worktree that already carries the number is printed
  and the command exits 0, so the command is safe to re-run; that check runs
  before the closed-issue refusal, since an issue closes while its worktree
  is still alive. It reads the same link `gwm list` shows, so a worktree
  attached by hand with `gwm link --issue` counts too.

  `--issue` is exclusive with the positional triple and with `--name`, the
  way `--name` already is: the mode is chosen explicitly, never inferred from
  how many arguments were supplied. Everything after the derivation is the
  existing `gwm create` path unchanged, `#{issue}` in `branch_pattern`
  included. An issue title is arbitrary text from the forge, so it reaches
  the slug through the same normaliser as a hand-typed `<desc>` rather than
  around it, and the echoed title, URL and labels are sanitised for the
  terminal.

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

- **A link can open in a terminal browser instead of leaving the terminal**
  ([#590](https://github.com/kbrdn1/gwm-cli/issues/590)). Every URL the TUI
  opens went to the system browser: the browse-links menu (`B`), the
  open-menu Issue and PR picks, a row in the rich PR/issue view, a CI check's
  details URL, `.` for the docs. On a tiling setup that means losing the
  workspace gwm is sitting in. The new `[tui] terminal_browser` names a
  command that renders the page in the terminal instead:

  ```toml
  [tui]
  terminal_browser = "w3m {url}"   # or lynx / carbonyl / browsh
  ```

  The `{url}` placeholder is optional: a bare `"w3m"` gets the URL appended
  as its last argument, which all four of those tools take anyway.

  **It is only consulted when a multiplexer is detected.** A terminal browser
  with nowhere to put it is worse than the system browser, so `$TMUX` /
  `$ZELLIJ` / `$HERDR_ENV` gate it, and the page opens in a new pane or tab
  beside gwm, at the level `[tui] mux_open_in` and `mux_pane_direction`
  already set for `t` and `o`. Where the container takes no command (herdr,
  a zellij tab, any `workspace`) it runs in the PTY overlay instead, so the
  browser still renders in the terminal, and the status bar names the backend
  that refused a pane. Anywhere else, including a browser that is not on
  `$PATH`, the system browser answers as it always has, with the reason on
  the status bar rather than silently.

  **A browser that places itself is launched rather than hosted**, via the
  companion `[tui] terminal_browser_open_in`:

  ```toml
  [tui]
  terminal_browser = "terminal-browser open {url} --split right"
  terminal_browser_open_in = "detached"   # default "overlay"
  ```

  Both shapes above host the browser, which assumes it draws inside the TTY
  it is handed. That holds for `w3m` and `lynx` and fails for one that
  renders through the terminal's image protocol: it positions against the
  real window, so in the PTY overlay it paints over the top-left corner of
  the screen whatever rect gwm passes, and in a gwm pane it splits twice
  because it splits on its own. `"detached"` launches the command and stops
  there. The two gates stay in front of it: no multiplexer still means the
  system browser, since placing itself means asking a multiplexer for a pane,
  and a missing binary still falls back. An unknown value errors at load.

  **Unset is the default and is exactly the behaviour up to 1.9**, on every
  platform. The key is also editable in the Settings panel under the **TUI**
  tab (`4`), where blanking it turns the feature back off.

  The URL is always one argument: the template is tokenised *before* the
  placeholder is substituted, so `w3m {url}` and `w3m "{url}"` are the same
  command and a URL's `?`, `&` and `#` cannot become shell syntax. Only
  absolute `http`/`https` URLs are passed on.

- **The Commits and Working Tree overlays say which worktree they are showing**
  ([#629](https://github.com/kbrdn1/gwm-cli/issues/629)). Both full-size
  listings painted their snapshot and nothing else: a commit graph that could
  be any branch's, a file tree that could be any worktree's. The modal title
  could not carry it either, being centred and therefore clipped from the
  left, and the commit overlay already spends its title on the row count.

  Each now carries a fixed row above its body: the branch for the commit
  listing, the worktree name and its path for the working tree. It is its own
  rect above the scroll region, not the first line of the listing, so it stays
  put while the body scrolls, and the Working Tree overlay renders it in its
  loading arm too, so the listing lands exactly where the loader was instead
  of jumping a line when the worker returns.

  The row resolves from the path the overlay pinned when it opened, not from
  the live selection: the auto-refresh moves the selection while an overlay is
  up, so the cursor can point at a different worktree than the rows on screen.
  It is an in-memory lookup, so the render path still shells out to nothing.

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

- **The rich Issue/PR view keeps its inline comments through a relist**
  ([#619](https://github.com/kbrdn1/gwm-cli/issues/619)). With the rich PR
  view open on a PR whose review threads had landed, a worktree relist (the
  periodic `tui.auto_refresh_secs` one, or an explicit `f`) emptied the
  thread cache, and the next PR result to land rebuilt the open view against
  an empty one. The inline comments disappeared from under the reader, and
  only closing and reopening the view, or refreshing it with `f`, brought
  them back.

  The view survives that same expiry for the PR itself because it renders
  its own snapshot of it; the threads were the one thing it read live from
  the cache. A relist now keeps the threads of the PR it is showing, which
  are as authoritative as the PR they hang from, and expires everyone
  else's as before. Re-requesting them on each tick instead would be fresher
  and worse: the section collapses to a loading line for the round trip,
  once per refresh interval, taking the reader's place in the comments with
  it. Refreshing the view with `f` still asks for them again.

- **`gwm create --issue` sanitises every value it echoes**
  ([#617](https://github.com/kbrdn1/gwm-cli/issues/617)). The branch type was
  printed raw on the summary line, and it reaches that line from two untrusted
  places: `--type` is argv, which clap hands through with its control bytes
  intact, and a type derived from the issue's labels is a key of
  `[issue_template.by_type]`, a string out of an unvetted repo's `.gwm.toml`.
  The echo happens before `BranchSpec::new_with_types` gets to reject the
  type, so validation was not the guard. All three values on those lines now
  go through `sanitise_for_terminal` rather than the diagnostic variant, which
  deliberately lets a newline through and would break a line these values are
  spliced into.

- **The CLI no longer runs on the 1 MiB stack Windows gives a process's main
  thread** ([#617](https://github.com/kbrdn1/gwm-cli/issues/617)).
  `Cli::parse` alone was sitting at that ceiling in a debug build: clap's
  derive expands one `Command` builder per subcommand and per argument into a
  single frame, and every `///` in `cli.rs` is a `long_help` string inside it.
  Adding three arguments to `gwm create` took the binary from "survives a
  1024 KiB stack, dies at 512" to "dies at 1024, survives 2048", so every
  `gwm.exe` invocation aborted with `STATUS_STACK_OVERFLOW` while macOS and
  Linux, which give main 8 MiB, stayed green.

  `main` now does nothing but spawn a worker with a 16 MiB stack and relay its
  exit status. Trimming doc comments back under the ceiling would have bought
  one release and handed the same failure to the next argument anyone adds;
  choosing the stack takes the ceiling out of the picture, and costs address
  space rather than memory, since a thread stack is reserved up front and
  committed page by page as it is used. `tests/main_stack_tests.rs` probes
  from a thread the size of the one Windows gives main, so the guard cannot
  pass vacuously on a Unix runner.

- **The selected worktree keeps the GitHub context that was fetched for it**
  ([#597](https://github.com/kbrdn1/gwm-cli/issues/597)). Standing on any row
  but the one the TUI opened on, `C` / `c` said "no CI checks to show: link a
  PR and fetch (F) first" for a worktree whose PR was linked and whose checks
  had already been fetched, and the rich Issue/PR view (`I`) refused for the
  same reason. `f` did not help: it refreshes the worktree list, not the
  GitHub layer, so only an `F` on that exact row filled the state back in.

  gwm was throwing away its own prefetch. It fetches every linked issue and
  PR at startup and on every relist, but the link re-read that runs on each
  selection change flushed the whole result cache and dropped any in-flight
  `gh` worker with it, so the prefetch died on the first `j`. That flush was
  a leftover: the cache has been keyed by number since #138, so it cannot
  serve one row's status for another, and a row that never fetched still
  reads as unfetched. It is now dropped only when the origin actually moves
  between two forge instances, which is the one case where a cached number
  means something else.

  The two verbs also stopped reading "nobody asked yet" as "nothing to show".
  A linked PR that has never been fetched is now fetched on the spot, on the
  same task spine, and reported as `fetching Pull request #61...`; one
  already in flight says the same without starting a second call; one whose
  probe failed shows what `gh` said instead of pointing at a fetch that had
  already run; and a PR that is fetched with an empty rollup says its checks
  have not been reported rather than naming a link and a fetch that are both
  already done. Only a row with nothing linked still gets the link hint.
  Workspace mode gains the most: it skips the bulk prefetch by design, so
  before this the verbs there were fed by nothing at all.

  Freshness is unchanged: a relist still expires every fetched status, so
  `tui.auto_refresh_secs` (60 by default) still bounds how stale one can be.
  That expiry moved ahead of the bulk prefetch's early returns, which is what
  gives workspace mode the same bound rather than none.
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
