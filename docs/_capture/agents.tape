# Agent sessions overlay (`a`) — one row per session on the selected
# worktree: pinned next to unpinned, active next to idle. Reads the demo's
# seeded artefact store through the GWM_AGENTS_HOME seam, never the real one.
# Run from the repo root (as generate.sh does): refresh-agents.sh is relative.
Output docs/_capture/.tmp/agents.gif
Set Shell "bash"
Set FontFamily "CaskaydiaCove Nerd Font Mono"
Set FontSize 15
# 1500 like the other TUI tapes. Widening the terminal does not help the one
# imperfection here (the pinned row's trailing `pinned` marker truncates to
# `pi…`): the overlay sizes itself, not to the terminal.
Set Width 1500
Set Height 860
Set Padding 22
Set Theme { "name": "gwm-grey", "background": "#2b2b2b", "foreground": "#d6d6d6", "cursor": "#d6d6d6", "selection": "#3d3d3d", "black": "#3a3a3a", "red": "#e06c75", "green": "#98c379", "yellow": "#e5c07b", "blue": "#61afef", "magenta": "#c678dd", "cyan": "#56b6c2", "white": "#d6d6d6", "brightBlack": "#6a6a6a", "brightRed": "#e06c75", "brightGreen": "#98c379", "brightYellow": "#e5c07b", "brightBlue": "#61afef", "brightMagenta": "#c678dd", "brightCyan": "#56b6c2", "brightWhite": "#ffffff" }

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
Type "gwm"
Enter
Sleep 3s
# 4 rows down is the dirty feat/#42 worktree, the one carrying two sessions
# (same row sidebar.tape selects).
Down@500ms 4
Sleep 1500ms
Type "a"
Sleep 2s
Screenshot docs/2.tui/_assets/agent-sessions.png
