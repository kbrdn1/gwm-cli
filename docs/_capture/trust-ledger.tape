# TOFU trust prompt — first `gwm create` on an untrusted .gwm.toml shows the
# bootstrap-surface summary + "Trust this .gwm.toml? [y/N/show]". We answer `n`,
# so payments-svc is never added to the ledger and the prompt re-arms next run.
Output docs/_capture/.tmp/trust.gif
Set Shell "bash"
Set FontFamily "CaskaydiaCove Nerd Font Mono"
Set FontSize 15
Set Width 900
Set Height 360
Set Padding 24
Set Theme { "name": "gwm-grey", "background": "#141414", "foreground": "#d6d6d6", "cursor": "#d6d6d6", "selection": "#3d3d3d", "black": "#3a3a3a", "red": "#e06c75", "green": "#98c379", "yellow": "#e5c07b", "blue": "#61afef", "magenta": "#c678dd", "cyan": "#56b6c2", "white": "#d6d6d6", "brightBlack": "#6a6a6a", "brightRed": "#e06c75", "brightGreen": "#98c379", "brightYellow": "#e5c07b", "brightBlue": "#61afef", "brightMagenta": "#c678dd", "brightCyan": "#56b6c2", "brightWhite": "#ffffff" }

Hide
Type `export PS1="$ "`
Enter
Type "cd /Users/kbrdn1/gwm-demo/payments-svc"
Enter
Type "clear"
Enter
Show
Sleep 400ms
Type "gwm create feat 4 stripe-gateway"
Enter
Sleep 2200ms
Screenshot docs/4.configuration/_assets/trust-ledger.png
Hide
Type "n"
Enter
Sleep 800ms
Sleep 1s
