---
title: docs/ — authoring conventions
description: How the gwm documentation tree is organised for Nuxt Content (or any other SSG).
navigation: false
---

# `docs/` — authoring conventions

This tree is the source of truth for the gwm user docs and the future static documentation site. It is structured to drop straight into [Nuxt Content](https://content.nuxt.com/) (or any SSG that follows the same numeric-prefix routing convention).

## layout

```
docs/
├── index.md                              # → /
├── 1.getting-started/                    # → /getting-started
│   ├── index.md
│   ├── 1.install.md                      # → /getting-started/install
│   ├── 2.first-worktree.md
│   └── 3.shell-init.md
├── 2.tui/                                # → /tui
├── 3.cli/                                # → /cli
├── 4.configuration/                      # → /configuration
├── 5.integrations/                       # → /integrations
├── 6.development/                        # → /development
└── 7.roadmap.md                          # → /roadmap
```

Numeric prefixes (`1.`, `2.`, `3.`) drive **sidebar ordering** in Nuxt Content and are stripped from the resulting URL. Add a new page anywhere in the tree by giving it the next free prefix in its parent folder.

## frontmatter contract

Every page (including section `index.md` files) carries this minimal frontmatter:

```yaml
---
title: <page title — rendered as <h1> and in <title>>
description: <one-sentence teaser, used for SEO and search>
---
```

Optional fields:

- `navigation.title` — short label for the sidebar when the full title is too long.
- `navigation.icon` — Iconify name (e.g. `lucide:terminal`) for SSGs that render section icons.
- `navigation: false` — hide the page from the auto-generated sidebar (use for this README only).

## links between docs pages

Use **repo-relative paths from this `docs/` root** so the same links resolve on GitHub and inside the future site:

```md
See [Configurable launchers](/tui/launchers) for the `[git_tui]` / `[review]` schema.
```

(Nuxt Content rewrites bare `/segment` paths against the content root; on GitHub they render as broken-but-readable cross-references — acceptable until the site is live, at which point a relative-link audit can lift them all in one pass.)

## images & assets

When pages need screenshots or diagrams, drop them under `docs/<section>/_assets/` and reference them with a relative path (`![keymap](./_assets/tui-keymap.png)`). Keep this README out of the generated sidebar via `navigation: false`.

## see also

- [`CONTRIBUTING.md`](../CONTRIBUTING.md) — branch / commit / PR conventions
- [`CHANGELOG.md`](../CHANGELOG.md) — release notes (root = `[Unreleased]`, per-version archives under `changelogs/`)
- [`examples/gwm.toml.example`](../examples/gwm.toml.example) — annotated config reference
