# First worktree — the 30-second tour: init a repo, seed a .gwm.toml from a
# preset, create the first worktree, open the TUI. Runs in a throwaway repo.
Output docs/1.getting-started/_assets/first-worktree.gif
Set Shell "bash"
Set FontFamily "CaskaydiaCove Nerd Font Mono"
Set FontSize 15
Set Width 1000
Set Height 720
Set Padding 22
Set Framerate 12
Set Theme { "name": "gwm-grey", "background": "#141414", "foreground": "#d6d6d6", "cursor": "#d6d6d6", "selection": "#3d3d3d", "black": "#3a3a3a", "red": "#e06c75", "green": "#98c379", "yellow": "#e5c07b", "blue": "#61afef", "magenta": "#c678dd", "cyan": "#56b6c2", "white": "#d6d6d6", "brightBlack": "#6a6a6a", "brightRed": "#e06c75", "brightGreen": "#98c379", "brightYellow": "#e5c07b", "brightBlue": "#61afef", "brightMagenta": "#c678dd", "brightCyan": "#56b6c2", "brightWhite": "#ffffff" }

Hide
Type `export PS1="$ "`
Enter
Type "rm -rf /Users/kbrdn1/gwm-demo/tour && mkdir -p /Users/kbrdn1/gwm-demo/tour"
Enter
Type "cd /Users/kbrdn1/gwm-demo/tour"
Enter
Type "clear"
Enter
Show
Sleep 500ms
Type@50ms "git init -q -b main && echo 'fn main() {}' > main.rs && git add -A && git commit -qm init"
Enter
Sleep 1600ms
Type@60ms "gwm init --preset rust"
Enter
Sleep 2200ms
Type@60ms "gwm create feat 1 health-check --allow-bootstrap"
Enter
Sleep 2600ms
Type@60ms "gwm"
Enter
Sleep 2600ms
Type "q"
Sleep 1000ms
