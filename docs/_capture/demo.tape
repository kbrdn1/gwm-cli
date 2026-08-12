# gwm - usage demo for the Terminal Trove listing.
# Worktree lifecycle in the TUI: agents (a), read a note (N), create (n) +
# bootstrap report, delete (d).
#
# The TUI is launched inside the hidden block and given time to paint before
# `Show`, so frame 0 is the interface rather than an empty prompt (issue #523).
# On GitHub, on X and in a Terminal Trove listing, frame 0 is the thumbnail: a
# black rectangle is what the reader decides on.
Output docs/_capture/demo.gif
Set Shell "bash"
Set FontFamily "CaskaydiaCove Nerd Font Mono"
Set FontSize 15
Set Width 1200
Set Height 680
Set Padding 20
Set Framerate 14
Set Theme { "name": "gwm-grey", "background": "#2b2b2b", "foreground": "#d6d6d6", "cursor": "#d6d6d6", "selection": "#3d3d3d", "black": "#3a3a3a", "red": "#e06c75", "green": "#98c379", "yellow": "#e5c07b", "blue": "#61afef", "magenta": "#c678dd", "cyan": "#56b6c2", "white": "#d6d6d6", "brightBlack": "#6a6a6a", "brightRed": "#e06c75", "brightGreen": "#98c379", "brightYellow": "#e5c07b", "brightBlue": "#61afef", "brightMagenta": "#c678dd", "brightCyan": "#56b6c2", "brightWhite": "#ffffff" }

Hide
Type `export PS1="$ " GWM_ALLOW_BOOTSTRAP=1 GWM_AGENTS_HOME="$HOME/gwm-demo/agents-home"`
Enter
# The fabricated agent artefacts age between `setup-demo.sh` and this
# recording, and past ACTIVE_WINDOW (5 min) they render "idle" grey. The
# freshness colour is half of what the AGENT column says, so refresh the
# mtimes here rather than depending on how recently the fixture was built.
#
# Through refresh-agents.sh rather than a blanket `touch`: since #524 the
# fixture also holds two sessions the other captures deliberately show as
# *idle*, and touching everything would turn them green. The script owns those
# ages, and its default is the same "everything active" this recording wants.
# Before the `cd` below, since the path is relative to the repo root.
Type "bash docs/_capture/refresh-agents.sh"
Enter
Type "cd /Users/kbrdn1/gwm-demo/acme-api"
Enter
Type "gwm remove feat-91-rate-limit-alerts --force --delete-branch >/dev/null 2>&1; git branch -D 'feat/#91-rate-limit-alerts' 'feat/#88-rate-limit-alerts' >/dev/null 2>&1; git worktree prune >/dev/null 2>&1; rm -rf .git/gwm/notes; clear"
Enter
Type "gwm"
Enter
Sleep 3000ms
Show

Sleep 1600ms

# Who is working where (#408): the AGENT column names the agent per row,
# `a` opens the full detected list. The sidebar pane below shows the pinned
# session only, which is why the table and the overlay both earn a beat.
Down 1
Sleep 1600ms
Type "a"
Sleep 2200ms
Escape
Sleep 1000ms

# The note: `N` opens it in a modal, `Esc` saves and closes (#515).
Type "N"
Sleep 900ms
Type@55ms "Rate limiter drops the burst allowance under load."
Sleep 400ms
Enter
Type@55ms "Check the 429 headers before opening the PR."
Sleep 1500ms
Escape
Sleep 1200ms

Type "n"
Sleep 1100ms
Type@130ms "91"
Sleep 600ms
Tab
Sleep 500ms
Type@100ms "rate-limit-alerts"
Sleep 900ms
Enter
Sleep 2700ms
Escape
Sleep 900ms
Type "/"
Sleep 500ms
Type@140ms "91"
Sleep 1000ms
Enter
Sleep 800ms
Type "d"
Sleep 1800ms
Type "y"
Sleep 2300ms
Escape
Sleep 1300ms
Type "q"
Sleep 700ms
