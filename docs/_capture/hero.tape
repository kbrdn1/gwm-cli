# Hero — full gwm TUI over the acme-api demo repo (worktree table + sidebar).
# Still PNG via `Screenshot`; the gif Output is a throwaway (vhs requires one).
# gwm default theme paints no background → inherits the neutral grey below.
Output docs/_capture/.tmp/hero.gif
Set Shell "bash"
Set FontFamily "CaskaydiaCove Nerd Font Mono"
Set FontSize 30
Set Width 2000
Set Height 1240
Set Padding 44
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
Down@500ms 4
Sleep 2s
Screenshot docs/2.tui/_assets/hero.png
Sleep 1s
