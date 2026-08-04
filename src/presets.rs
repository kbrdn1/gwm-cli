//! Built-in `.gwm.toml` presets for `gwm init --preset <name>` (issue #37).
//!
//! Every new project repeats the same `gwm init` → trim → customise dance:
//! Laravel wants the AWS-RDS guard + composer install + a `vendor/`
//! no-symlink, Nuxt wants `node_modules/` no-symlink + a package-manager
//! aware install, Rust wants `target/` no-symlink + cargo fetch. These
//! patterns recur across users yet every repo re-derives them.
//!
//! A preset seeds an opinionated `.gwm.toml` for a known stack. The bodies
//! are TOML templates embedded in the binary via [`include_str!`], kept in
//! sync with `examples/presets/<name>.toml`. The `generic` preset's body is
//! the documented `examples/gwm.toml.example`, so `gwm init` with no preset
//! stays byte-for-byte identical to its historical behaviour.

/// A built-in config preset: a canonical name, optional aliases, a one-line
/// description for `--list-presets`, and the embedded `.gwm.toml` body.
#[derive(Debug, Clone, Copy)]
pub struct Preset {
  /// Canonical name accepted by `--preset <name>`.
  pub name: &'static str,
  /// Alternative names that resolve to this same preset (e.g. `nuxt` → `node`).
  pub aliases: &'static [&'static str],
  /// One-line summary shown by `gwm init --list-presets`.
  pub description: &'static str,
  /// The `.gwm.toml` body this preset writes.
  pub body: &'static str,
}

/// The generic preset's body is the documented example shipped with the
/// binary — identical to what `gwm init` (no preset) has always written.
pub const GENERIC_BODY: &str = include_str!("../examples/gwm.toml.example");

/// The built-in preset registry. Order is the display order of
/// `--list-presets` (generic first, then stacks alphabetically).
const PRESETS: &[Preset] = &[
  Preset {
    name: "generic",
    aliases: &[],
    description: "The fully-documented default template (same as `gwm init`).",
    body: GENERIC_BODY,
  },
  Preset {
    name: "go",
    aliases: &[],
    description: "Go module: bin/ no-symlink + `go mod download`.",
    body: include_str!("../examples/presets/go.toml"),
  },
  Preset {
    name: "laravel",
    aliases: &[],
    description: "Laravel: env copies + AWS-RDS guard + vendor/ no-symlink + composer install.",
    body: include_str!("../examples/presets/laravel.toml"),
  },
  Preset {
    name: "node",
    aliases: &["nuxt"],
    description: "Node / Nuxt: node_modules/ no-symlink + bun-or-npm install.",
    body: include_str!("../examples/presets/node.toml"),
  },
  Preset {
    name: "python-uv",
    aliases: &[],
    description: "Python (uv): .venv/ no-symlink + `uv sync`.",
    body: include_str!("../examples/presets/python-uv.toml"),
  },
  Preset {
    name: "rust",
    aliases: &[],
    description: "Rust crate: target/ no-symlink + `cargo fetch`.",
    body: include_str!("../examples/presets/rust.toml"),
  },
  Preset {
    name: "symfony",
    aliases: &[],
    description: "Symfony: .env.local copy + AWS-RDS guard + vendor/ and var/ no-symlink + composer install.",
    body: include_str!("../examples/presets/symfony.toml"),
  },
];

/// All built-in presets, in display order.
pub fn all() -> &'static [Preset] {
  PRESETS
}

/// Resolve a preset by canonical name or alias. Returns `None` for an
/// unknown name so the caller can surface a `--list-presets` hint.
pub fn lookup(name: &str) -> Option<&'static Preset> {
  PRESETS.iter().find(|p| p.name == name || p.aliases.contains(&name))
}
