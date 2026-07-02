# Live fuzzy filter — `/` narrows the worktree list as you type.
Output docs/2.tui/_assets/filter.gif
Set Shell "bash"
Set FontFamily "CaskaydiaCove Nerd Font Mono"
Set FontSize 15
Set Width 1320
Set Height 720
Set Padding 20
Set Framerate 12
Set Theme { "name": "gwm-grey", "background": "#2b2b2b", "foreground": "#d6d6d6", "cursor": "#d6d6d6", "selection": "#3d3d3d", "black": "#3a3a3a", "red": "#e06c75", "green": "#98c379", "yellow": "#e5c07b", "blue": "#61afef", "magenta": "#c678dd", "cyan": "#56b6c2", "white": "#d6d6d6", "brightBlack": "#6a6a6a", "brightRed": "#e06c75", "brightGreen": "#98c379", "brightYellow": "#e5c07b", "brightBlue": "#61afef", "brightMagenta": "#c678dd", "brightCyan": "#56b6c2", "brightWhite": "#ffffff" }

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
Type "/"
Sleep 700ms
Type@180ms "webh"
Sleep 1800ms
Backspace@140ms 4
Sleep 500ms
Type@180ms "fix"
Sleep 1800ms
Escape
Sleep 900ms
