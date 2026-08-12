# Compact layout (issue #545) — same screen as hero.png, `[tui] compact = true`.
# The config is injected by generate.sh before this tape runs, the same way the
# theme gallery injects each preset. Geometry matches hero.tape on purpose: the
# two captures sit side by side in the docs, so any difference between them has
# to come from the mode and not from the window.
#
# The injected config pins `[theme] preset = "claude-dark"` rather than
# inheriting whatever the capture machine has in ~/.config/gwm — this is the
# first capture whose subject is a *background* role, so the terminal
# background below is the palette's own (#1a1a1a) instead of the neutral grey
# the other tapes use. On the neutral grey the header fill lands within a
# couple of shades of the terminal and the mode looks like it did nothing.
Output docs/_capture/.tmp/compact.gif
Set Shell "bash"
Set FontFamily "CaskaydiaCove Nerd Font Mono"
Set FontSize 15
Set Width 1500
Set Height 820
Set Padding 22
Set Theme { "name": "gwm-claude-dark", "background": "#1a1a1a", "foreground": "#d6d6d6", "cursor": "#d6d6d6", "selection": "#3d3d3d", "black": "#3a3a3a", "red": "#e06c75", "green": "#98c379", "yellow": "#e5c07b", "blue": "#61afef", "magenta": "#c678dd", "cyan": "#56b6c2", "white": "#d6d6d6", "brightBlack": "#6a6a6a", "brightRed": "#e06c75", "brightGreen": "#98c379", "brightYellow": "#e5c07b", "brightBlue": "#61afef", "brightMagenta": "#c678dd", "brightCyan": "#56b6c2", "brightWhite": "#ffffff" }

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
Sleep 1500ms
Screenshot docs/2.tui/_assets/compact.png
