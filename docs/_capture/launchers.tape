# Git-TUI launcher in the embedded PTY overlay (`l`) — lazygit driven by a
# real pseudo-terminal inside gwm, the worktree list still framing it. This is
# the thing `tui/launchers` describes and prose cannot show: no alt-screen
# swap, no suspend, the overlay border around a live child process.
Output docs/_capture/.tmp/launchers.gif
Set Shell "bash"
Set FontFamily "CaskaydiaCove Nerd Font Mono"
Set FontSize 15
Set Width 1000
Set Height 900
Set Padding 22
Set Theme { "name": "gwm-grey", "background": "#141414", "foreground": "#d6d6d6", "cursor": "#d6d6d6", "selection": "#3d3d3d", "black": "#3a3a3a", "red": "#e06c75", "green": "#98c379", "yellow": "#e5c07b", "blue": "#61afef", "magenta": "#c678dd", "cyan": "#56b6c2", "white": "#d6d6d6", "brightBlack": "#6a6a6a", "brightRed": "#e06c75", "brightGreen": "#98c379", "brightYellow": "#e5c07b", "brightBlue": "#61afef", "brightMagenta": "#c678dd", "brightCyan": "#56b6c2", "brightWhite": "#ffffff" }

Hide
Type `export PS1="$ "`
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
# the dirty feat/#42 worktree: lazygit then has staged and unstaged files to draw
Down@500ms 4
Sleep 1500ms
Type "l"
# Nothing is sent between here and the shot on purpose. lazygit greets a NEW
# version with a full-screen "Breaking Changes" modal that covers this whole
# capture, and there is no keystroke that dismisses it and is harmless when it
# is absent: `Escape` falls through to gwm and closes the overlay, `Enter`
# falls through to lazygit's file panel. Open lazygit once by hand after
# upgrading it, then regenerate.
Sleep 5s
Screenshot docs/2.tui/_assets/launchers.png
Sleep 1s
