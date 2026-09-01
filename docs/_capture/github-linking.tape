# GitHub linking: the Issue·PR sidebar pane with a live, gh-detected PR.
# Captured against the real gwm-cli repo, because the acme-api demo has no
# remote and this pane needs one. It reads the checkout's current branch, so it
# wants to run from a branch whose PR is open; `F` fetches, then screenshot.
# No filter: the pane follows the selected row, and the default selection is the
# checkout the tape cds into.
#
# That checkout comes from GWM_CAPTURE_REPO, which generate.sh sets to the tree
# it is capturing; the fallback keeps a bare `vhs docs/_capture/github-linking.tape`
# working. Hardcoded, the path pointed at the maintainer's trunk whatever
# checkout the run was launched from, which is the wrong tree as soon as a
# release is cut in a worktree. The Working Tree pane photographs whatever is
# uncommitted there, which is why generate.sh refuses to run this against a
# dirty tree (#631).
Output docs/_capture/.tmp/github-linking.gif
Set Shell "bash"
Set FontFamily "CaskaydiaCove Nerd Font Mono"
Set FontSize 30
Set Width 1980
Set Height 1600
Set Padding 44
Set Theme { "name": "gwm-grey", "background": "#141414", "foreground": "#d6d6d6", "cursor": "#d6d6d6", "selection": "#3d3d3d", "black": "#3a3a3a", "red": "#e06c75", "green": "#98c379", "yellow": "#e5c07b", "blue": "#61afef", "magenta": "#c678dd", "cyan": "#56b6c2", "white": "#d6d6d6", "brightBlack": "#6a6a6a", "brightRed": "#e06c75", "brightGreen": "#98c379", "brightYellow": "#e5c07b", "brightBlue": "#61afef", "brightMagenta": "#c678dd", "brightCyan": "#56b6c2", "brightWhite": "#ffffff" }

Hide
Type `export PS1="$ "`
Enter
Type "cd ${GWM_CAPTURE_REPO:-/Users/kbrdn1/Projects/Perso/gwm-cli}"
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
