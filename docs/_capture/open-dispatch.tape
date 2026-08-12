# Open dispatch (`o`) — a shell in the embedded PTY overlay, already `cd`-ed
# into the selected worktree. The point `tui/open-dispatch` makes in prose is
# exactly this: the terminal opens *in* the worktree, without leaving the TUI.
Output docs/_capture/.tmp/open-dispatch.gif
Set Shell "bash"
Set FontFamily "CaskaydiaCove Nerd Font Mono"
Set FontSize 15
Set Width 1500
Set Height 700
Set Padding 22
Set Theme { "name": "gwm-grey", "background": "#2b2b2b", "foreground": "#d6d6d6", "cursor": "#d6d6d6", "selection": "#3d3d3d", "black": "#3a3a3a", "red": "#e06c75", "green": "#98c379", "yellow": "#e5c07b", "blue": "#61afef", "magenta": "#c678dd", "cyan": "#56b6c2", "white": "#d6d6d6", "brightBlack": "#6a6a6a", "brightRed": "#e06c75", "brightGreen": "#98c379", "brightYellow": "#e5c07b", "brightBlue": "#61afef", "brightMagenta": "#c678dd", "brightCyan": "#56b6c2", "brightWhite": "#ffffff" }

Hide
Type `export PS1="$ "`
Enter
# The overlay spawns `$SHELL` (tui/mod.rs), which on this machine is a zsh with
# a personal prompt: battery, icons, and a slow startup that swallowed the
# first keystrokes. Pin it to bash so the capture shows gwm's overlay, not the
# operator's dotfiles.
Type `export SHELL=/bin/bash PS1="$ "`
Enter
Type "cd /Users/kbrdn1/gwm-demo/acme-api"
Enter
Type "clear"
Enter
Show
Sleep 400ms
Type "gwm"
Enter
Sleep 3s
# the dirty feat/#42 worktree, so `git status -sb` in the overlay has content
Down@500ms 4
Sleep 1500ms
Type "o"
# Typing before the child shell has drawn its prompt leaves the command echoed
# but never run.
Sleep 4s
Type@80ms "git status -sb"
Enter
Sleep 5s
Screenshot docs/2.tui/_assets/open-dispatch.png
