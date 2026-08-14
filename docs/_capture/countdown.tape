# Safety countdown — the confirm-delete overlay with `D` armed:
#   d (open) → D (arm branch delete) → y (arm the 3s countdown: progress bar +
#   animated spinner) → Esc well before it fires. Nothing is deleted (we cancel
#   ~1s early, and setup-demo rebuilds docs/#71 anyway).
Output docs/2.tui/_assets/countdown.gif
Set Shell "bash"
Set FontFamily "CaskaydiaCove Nerd Font Mono"
Set FontSize 30
Set Width 2000
Set Height 1240
Set Padding 40
Set Framerate 14
Set Theme { "name": "gwm-grey", "background": "#141414", "foreground": "#d6d6d6", "cursor": "#d6d6d6", "selection": "#3d3d3d", "black": "#3a3a3a", "red": "#e06c75", "green": "#98c379", "yellow": "#e5c07b", "blue": "#61afef", "magenta": "#c678dd", "cyan": "#56b6c2", "white": "#d6d6d6", "brightBlack": "#6a6a6a", "brightRed": "#e06c75", "brightGreen": "#98c379", "brightYellow": "#e5c07b", "brightBlue": "#61afef", "brightMagenta": "#c678dd", "brightCyan": "#56b6c2", "brightWhite": "#ffffff" }

Hide
Type `export PS1="$ "`
Enter
Type "cd /Users/kbrdn1/gwm-demo/acme-api"
Enter
Type "clear"
Enter
Show
Sleep 500ms
Type "gwm"
Enter
Sleep 2500ms
Down@450ms 2
Sleep 700ms
Type "d"
Sleep 1200ms
Type "D"
Sleep 1000ms
Type "y"
Sleep 2000ms
Escape
Sleep 900ms
