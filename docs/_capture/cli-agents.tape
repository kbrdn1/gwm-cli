# `gwm agents` — sessions grouped per worktree, with agent, freshness,
# last activity, id and name. Reads the demo's seeded artefact store through
# the GWM_AGENTS_HOME seam, never the operator's real one.
# Run from the repo root (as generate.sh does): refresh-agents.sh is relative.
Output docs/_capture/.tmp/cli-agents.gif
Set Shell "bash"
Set FontFamily "CaskaydiaCove Nerd Font Mono"
Set FontSize 30
Set Width 2440
Set Height 600
Set Padding 44
Set Theme { "name": "gwm-grey", "background": "#141414", "foreground": "#d6d6d6", "cursor": "#d6d6d6", "selection": "#3d3d3d", "black": "#3a3a3a", "red": "#e06c75", "green": "#98c379", "yellow": "#e5c07b", "blue": "#61afef", "magenta": "#c678dd", "cyan": "#56b6c2", "white": "#d6d6d6", "brightBlack": "#6a6a6a", "brightRed": "#e06c75", "brightGreen": "#98c379", "brightYellow": "#e5c07b", "brightBlue": "#61afef", "brightMagenta": "#c678dd", "brightCyan": "#56b6c2", "brightWhite": "#ffffff" }

Hide
Type `export PS1="$ "`
Enter
Type "export GWM_AGENTS_HOME=$HOME/gwm-demo/agents-home"
Enter
Type "bash docs/_capture/refresh-agents.sh"
Enter
Type "cd /Users/kbrdn1/gwm-demo/acme-api"
Enter
Type "clear"
Enter
Show
Sleep 400ms
Type@60ms "gwm agents"
Enter
Sleep 2s
Screenshot docs/3.cli/_assets/cli-agents.png
Sleep 1s
