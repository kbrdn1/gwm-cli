# Bootstrap step report: the ✓ · ! ✗ lines printed after `gwm create`.
# Creates a throwaway worktree, screenshots the report, then removes it so the
# demo worktree set is unchanged for every other capture.
#
# The teardown deletes the branch as well as the worktree. `gwm remove` keeps it
# on purpose (CONTRIBUTING: the branch is the artefact), which makes this the
# one tape that is not idempotent: a second run in the same fixture greets
# `gwm create` with `branch 'feat/#88-add-metrics' already exists` and captures
# the error instead of the report. Hit for real when generate.sh gained a retry:
# the retry ran, the file was written, the guard passed, and the shipped PNG was
# the failure. Torn down both ends, so the tape survives a rerun whatever
# state the last one left behind.
Output docs/_capture/.tmp/bootstrap.gif
Set Shell "bash"
Set FontFamily "CaskaydiaCove Nerd Font Mono"
Set FontSize 15
Set Width 900
Set Height 460
Set Padding 24
Set Theme { "name": "gwm-grey", "background": "#141414", "foreground": "#d6d6d6", "cursor": "#d6d6d6", "selection": "#3d3d3d", "black": "#3a3a3a", "red": "#e06c75", "green": "#98c379", "yellow": "#e5c07b", "blue": "#61afef", "magenta": "#c678dd", "cyan": "#56b6c2", "white": "#d6d6d6", "brightBlack": "#6a6a6a", "brightRed": "#e06c75", "brightGreen": "#98c379", "brightYellow": "#e5c07b", "brightBlue": "#61afef", "brightMagenta": "#c678dd", "brightCyan": "#56b6c2", "brightWhite": "#ffffff" }

Hide
Type `export PS1="$ "`
Enter
Type "cd /Users/kbrdn1/gwm-demo/acme-api"
Enter
Type "gwm remove feat-88-add-metrics --force >/dev/null 2>&1; git branch -D 'feat/#88-add-metrics' >/dev/null 2>&1; true"
Enter
Sleep 800ms
Type "clear"
Enter
Show
Sleep 400ms
Type "gwm create feat 88 add-metrics --allow-bootstrap"
Enter
Sleep 2500ms
Screenshot docs/4.configuration/_assets/bootstrap.png
Hide
Type "gwm remove feat-88-add-metrics --force; git branch -D 'feat/#88-add-metrics'"
Enter
Sleep 1200ms
Sleep 1s
