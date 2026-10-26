# Provenance for a capture run: what `gwm --version` answers *through vhs*.
#
# The version chip in the TUI header is painted from the binary vhs resolves,
# not from the tree, so a stale `gwm` earlier on PATH renders a plausible
# capture of the wrong release (#631). This tape asks the question in the same
# shell every other tape runs in, and writes the answer as text — no pixels, so
# the guard in `tests/docs_assets_tests.rs` can read it back without OCR.
#
# `generate.sh` runs this first and aborts the run when the answer is not the
# version in Cargo.toml, or when the file it resolved is not the one cargo just
# built. It writes under `.tmp/`, which is gitignored: the
# committed copy is made at the end, because `github-linking.tape`
# photographs this repo's working tree and must find it clean.
Output docs/_capture/.tmp/version-stamp.gif
Set Shell "bash"
Set FontFamily "CaskaydiaCove Nerd Font Mono"
Set FontSize 30
Set Width 1980
Set Height 400
Set Padding 44

Hide
Type `export PS1="$ "`
Enter
Type "gwm --version > docs/_capture/.tmp/version.txt"
Enter
# The version alone cannot separate this tree's build from another one at the
# same version, which is the shape the v1.10.0 near miss actually had: 175
# commits behind, same number. The path can, so it is reported too, in a file
# of its own because the committed stamp must stay one line and hold no
# machine path.
Type "command -v gwm > docs/_capture/.tmp/which.txt"
Enter
Show
# The redirection completes asynchronously with respect to vhs exiting, the
# same race every `Screenshot` carries a `Sleep` for: without this the file
# lands about two runs in three.
Sleep 2s
