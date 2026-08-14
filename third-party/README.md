# `third-party/`: notices for the libraries compiled into the binary

gwm links two C libraries **statically**. There is no `libgit2.so` or
`libz.so.1` for the package manager to resolve at install time, and no separate
package carrying their terms: the code is inside the `gwm` executable, so their
notices have to travel with every artefact this project distributes.

That is a condition, not a courtesy. libgit2 is GPLv2 **with a linking
exception**, and the exception is what makes a permissively-licensed binary
possible at all:

> In addition to the permissions in the GNU General Public License, the authors
> give you unlimited permission to link the compiled version of this library
> into combinations with other programs, and to distribute those combinations
> without any restriction coming from the use of this file.

Without the notice accompanying the distribution, the permission it grants is
not the one being relied on. zlib's terms are permissive but explicit on the
same point: its notice may not be removed or altered from any source
distribution.

Neither obligation has anything to do with the project's own license. Both were
unmet while gwm was MIT-only, and both are unmet the same way under
`MIT OR Apache-2.0`; #573 changed the project's license and left this alone on
purpose, and #577 is this.

## What is vendored, and from where

| File | Library | Comes from | Recorded version |
|---|---|---|---|
| `libgit2-COPYING` | libgit2 | `libgit2-sys` crate, `libgit2/COPYING` | `libgit2-sys 0.18.7+1.9.6` (libgit2 1.9.6) |
| `zlib-LICENSE` | zlib | `libz-sys` crate, `src/zlib/LICENSE` | `libz-sys 1.1.29` |

Both are byte-for-byte copies. `tests/third_party_notices_tests.rs` pins each
one by git blob id, so an edit reddens the suite:

```bash
git hash-object third-party/libgit2-COPYING   # 80788a3ed790689b5b30918d17ec67ccd24e7a20
git hash-object third-party/zlib-LICENSE      # b7a69d058e616651eae27b3f90c0b7fd36c099b2
```

## Refreshing after a dependency bump

The versions in the table above are checked against `Cargo.lock`. Bumping
`libgit2-sys` or `libz-sys` reddens
`the_recorded_provenance_matches_the_lockfile`, which is the point: a bump
moves the vendored library without touching any file a reviewer reads, so
without that guard the notice here would quietly go on describing a version the
binary no longer contains.

To refresh, take the notice out of the crate source cargo actually built, not
out of the library's upstream repository. The two diverge: `libgit2-sys` vendors
a pinned libgit2 tree, and `git clone`-ing libgit2 gives you whatever `main`
holds today.

```bash
# the crate source cargo unpacked for the locked version
SRC=$(find "${CARGO_HOME:-$HOME/.cargo}/registry/src" -maxdepth 1 -type d -name 'libgit2-sys-*' | tail -1)
cp "$SRC/libgit2/COPYING" third-party/libgit2-COPYING

SRC=$(find "${CARGO_HOME:-$HOME/.cargo}/registry/src" -maxdepth 1 -type d -name 'libz-sys-*' | tail -1)
cp "$SRC/src/zlib/LICENSE" third-party/zlib-LICENSE
```

Then update the table above, and the two blob ids in
`tests/third_party_notices_tests.rs`, from the new files.

## Where they ship

Both files are staged into every release archive by `release.yml` and
`pre-release.yml`, listed in the `.deb` and `.rpm` assets in `Cargo.toml`,
installed by the AUR `PKGBUILD`, and included in the published crate. Each of
those five surfaces has a guard in
`tests/third_party_notices_tests.rs`; dropping one reddens the suite rather than
shipping an artefact that is missing a notice.

This directory is only for libraries **compiled into** the binary. A tool gwm
shells out to (`git`, `gh`, `glab`, `lazygit`, an editor) is a separate program
the user installed themselves, and carries its own terms with its own package.
