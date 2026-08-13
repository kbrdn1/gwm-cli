# GitHub linking: the Issue·PR sidebar pane with a live, gh-detected PR.
# Captured against the real gwm-cli repo, because the acme-api demo has no
# remote and this pane needs one. It reads the checkout's current branch, so it
# wants to run from a branch whose PR is open; `F` fetches, then screenshot.
# No filter: the pane follows the selected row, and the default selection is the
# checkout the tape cds into.
Output docs/_capture/.tmp/github-linking.gif
Set Shell "bash"
Set FontFamily "CaskaydiaCove Nerd Font Mono"
Set FontSize 15
Set Width 1000
Set Height 820
Set Padding 22
Set Theme { "name": "gwm-grey", "background": "#141414", "foreground": "#d6d6d6", "cursor": "#d6d6d6", "selection": "#3d3d3d", "black": "#3a3a3a", "red": "#e06c75", "green": "#98c379", "yellow": "#e5c07b", "blue": "#61afef", "magenta": "#c678dd", "cyan": "#56b6c2", "white": "#d6d6d6", "brightBlack": "#6a6a6a", "brightRed": "#e06c75", "brightGreen": "#98c379", "brightYellow": "#e5c07b", "brightBlue": "#61afef", "brightMagenta": "#c678dd", "brightCyan": "#56b6c2", "brightWhite": "#ffffff" }

Hide
Type `export PS1="$ "`
Enter
Type "cd /Users/kbrdn1/Projects/Perso/gwm-cli"
Enter
Type "clear"
Enter
Show
Sleep 400ms
Type "gwm"
Enter
Sleep 3s
Type "F"
Sleep 6000ms
Screenshot docs/5.integrations/_assets/github-linking.png
Sleep 1s
