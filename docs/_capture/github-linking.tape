# GitHub linking: the Issue·PR sidebar pane with a live, gh-detected PR.
# Captured against the real gwm-cli repo, because the acme-api demo has no
# remote and this pane needs one. It reads the checkout's current branch, so it
# wants to run from a branch whose PR is open; `F` fetches, then screenshot.
# No filter: the pane follows the selected row, and that row is the repo's main
# checkout, not the directory the tape cds into. Measured on a run launched from
# a clean linked worktree: the capture came back on the trunk's branch, with the
# trunk's PR and the trunk's uncommitted files.
#
# The path comes from GWM_CAPTURE_REPO, which generate.sh sets to the trunk of
# the repo it is capturing, so the cd lands on the tree the script gated as
# clean; the fallback keeps a bare `vhs docs/_capture/github-linking.tape`
# working, on the one machine that path exists (#631).
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
Type `cd "${GWM_CAPTURE_REPO:-/Users/kbrdn1/Projects/Perso/gwm-cli}"`
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
