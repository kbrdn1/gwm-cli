# Keymap editor — the Settings panel (`4`) on its `Keys` tab: the live keymap
# editor listing every rebindable global + modal binding with source badges.
# Tabs cycle Theme → Worktree → TUI → Keys, so three `Tab`s land on Keys.
Output docs/_capture/.tmp/keymap.gif
Set Shell "bash"
Set FontFamily "CaskaydiaCove Nerd Font Mono"
Set FontSize 15
Set Width 1240
Set Height 690
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
Type "4"
Sleep 1200ms
Tab@450ms 3
Sleep 1500ms
Screenshot docs/2.tui/_assets/keymap.png
Sleep 1s
