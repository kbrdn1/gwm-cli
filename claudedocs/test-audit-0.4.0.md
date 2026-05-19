# Test-suite audit — v0.4.0

Closes part of [#57](https://github.com/kbrdn1/gwm-cli/issues/57).

Walk of every test under `tests/` (193 total at v0.4.0), classified into
the three buckets defined by the issue:

1. **drives** — exercises a real production code path. Failure reveals
   a regression in the system under test.
2. **sentinel** — pinned exclusively to catch a previously-introduced
   bug from reappearing. Annotated inline with `// regression: <one-line>`
   so a future reader can identify the target without git-archaeology.
3. **dead** — drifts from intent: asserts a constant against itself,
   restates coverage another test already provides, or pins a behaviour
   we have since changed. Deleted in this PR.

## Summary

| Bucket               | Count | Δ vs v0.4.0 |
|:---------------------|------:|------------:|
| #1 drives            |   179 |           — |
| #2 sentinel (kept)   |    11 |    annotated|
| #3 dead (deleted)    |     3 |          −3 |
| **Total after audit**| **190** | **−3**    |

Target band per the issue: `[180, 195]`. We land on `190`, holding most
of the growth (193 → 190) while clearing the three tests that were
asserting against themselves or duplicating positive coverage.

## Deletions

All three are bucket #3 — deleted in this PR. Each deletion commit cites
the consolidating positive test that already subsumes the path.

| File                       | Test                                                  | Why dead | Subsumed by |
|:---------------------------|:------------------------------------------------------|:---------|:------------|
| `bootstrap_when_tests.rs`  | `evaluator_is_pure_for_a_given_cwd`                    | A "sanity probe" calling `evaluate_when(expr, cwd)` twice with the same args and asserting equality. The function is a pure read-over-FS-and-env predicate; the test pins purity that already follows from the signature. If `evaluate_when` ever mutated process state between calls, no realistic regression path lands in this test before crashing the surrounding suite. | The 27 positive evaluator tests next to it — each calls `evaluate_when` once on a controlled tempdir and asserts the boolean result. Any non-deterministic behaviour would show up there, not in a self-equality check. |
| `doctor_tests.rs`          | `prunable_check_detail_uses_singular_plural_correctly` | Constructs a `gwm::doctor::Check::warning(...)` *by hand* with the literal string `"1 prunable entry: feat-12-old"`, then asserts the string does not contain `entrie(`. The doctor's pluralisation logic is never invoked — the test is asserting its own input against itself. | The pluralisation contract is enforced where it lives, in `src/doctor.rs::pluralise_prunable_count` (and the existing `fresh_repo_has_no_prunable_worktrees` end-to-end path). If `pluralise_prunable_count` regressed, it would surface in any real `gwm doctor` run with one prunable worktree. |
| `cli_binary.rs`            | `shell_init_posix_does_not_use_paren_function_syntax`  | Asserts `gcd()` is **not** in the `shell-init` output. But `shell_init_bash_emits_gcd_function` and `shell_init_zsh_emits_gcd_function` already assert `function gcd` **is** present — for a realistic regression the implementation would switch from one form to the other, not emit both. The negative assertion duplicates the contract pinned by the positives. | `shell_init_bash_emits_gcd_function` + `shell_init_zsh_emits_gcd_function` (positive `function gcd` assertion). Together they pin the keyword form unambiguously; the explosive `gcd()` form cannot satisfy them. |

## Per-file classification

### `tests/bootstrap_tests.rs` — 13 tests, **all drives**

| Test                                            | Bucket  |
|:------------------------------------------------|:--------|
| `copy_step_happy_path`                          | drives  |
| `copy_step_skipped_when_dest_exists`            | drives  |
| `copy_step_required_source_missing_fails`       | drives  |
| `copy_step_optional_source_missing_skipped`     | drives  |
| `copy_step_inline_fallback_writes_content`      | drives  |
| `guard_abort_blocks_copy`                       | drives  |
| `guard_seed_from_example_substitutes`           | drives  |
| `guard_does_not_trip_on_safe_content`           | drives  |
| `no_symlink_removes_existing_symlink`           | drives  |
| `command_when_file_exists_skips_if_missing`     | drives  |
| `command_runs_when_condition_satisfied`         | drives  |
| `command_failure_recorded`                      | drives  |
| `command_env_is_propagated`                     | drives  |

### `tests/bootstrap_when_tests.rs` — 28 tests → 27 after audit

| Test                                                       | Bucket   | Notes |
|:-----------------------------------------------------------|:---------|:------|
| `file_exists_true_when_target_present`                     | drives   ||
| `file_exists_false_when_target_missing`                    | drives   ||
| `cmd_exists_true_for_sh`                                   | drives   ||
| `cmd_exists_false_for_made_up_binary`                      | drives   ||
| `env_set_true_for_path`                                    | drives   ||
| `env_set_false_for_unset_var`                              | drives   ||
| `env_eq_true_when_value_matches`                           | drives   ||
| `env_eq_false_when_value_mismatch`                         | drives   ||
| `env_eq_false_when_var_missing`                            | drives   ||
| `glob_exists_true_when_pattern_matches_in_root`            | drives   ||
| `glob_exists_true_for_recursive_pattern`                   | drives   ||
| `glob_exists_false_when_no_match`                          | drives   ||
| `not_inverts_file_exists_true`                             | drives   ||
| `not_inverts_file_exists_false`                            | drives   ||
| `and_true_when_both_sides_true`                            | drives   ||
| `and_false_when_left_false`                                | drives   ||
| `and_false_when_right_false`                               | drives   ||
| `or_true_when_left_true`                                   | drives   ||
| `or_true_when_right_true`                                  | drives   ||
| `or_false_when_both_sides_false`                           | drives   ||
| `precedence_not_binds_tighter_than_and`                    | drives   ||
| `precedence_and_binds_tighter_than_or`                     | drives   ||
| `precedence_and_binds_tighter_than_or_negative`            | drives   ||
| `extra_whitespace_around_operators_is_tolerated`           | drives   ||
| `unicode_whitespace_inside_atom_is_trimmed`                | sentinel | regression: NBSP-prefixed atoms forwarded to `Path::join` pre `.trim()` fix |
| `unicode_path_does_not_panic`                              | sentinel | regression: `tokenize_when` byte-slicing on multibyte UTF-8 paths |
| `unknown_keyword_still_defaults_to_true`                   | drives   | (forward-compat contract) |
| ~~`evaluator_is_pure_for_a_given_cwd`~~                    | **dead** | deleted — see *Deletions* |

### `tests/cli_binary.rs` — 31 tests → 30 after audit

| Test                                                       | Bucket   | Notes |
|:-----------------------------------------------------------|:---------|:------|
| `help_prints_subcommands`                                  | drives   | canary per CONTRIBUTING.md |
| `switch_alias_s_resolves_to_switch`                        | drives   ||
| `top_level_help_advertises_switch_alias`                   | drives   ||
| `switch_outside_git_repo_fails`                            | drives   ||
| `doctor_on_fresh_repo_prints_checks`                       | drives   ||
| `doctor_outside_git_repo_fails`                            | drives   ||
| `doctor_exits_two_on_invalid_config`                       | drives   ||
| `version_flag`                                             | drives   ||
| `types_lists_branch_types`                                 | drives   ||
| `create_outside_git_repo_fails`                            | drives   | exercises `list` outside a repo (mis-named) |
| `completions_zsh_emits_compdef_header`                     | drives   ||
| `completions_bash_emits_complete_directive`                | drives   ||
| `completions_fish_emits_complete_directive`                | drives   ||
| `completions_powershell_emits_register_block`              | drives   ||
| `completions_rejects_unknown_shell`                        | drives   ||
| `list_format_names_emits_one_name_per_line`                | drives   ||
| `cd_unknown_pattern_fails_with_not_found`                  | drives   ||
| `cd_outside_git_repo_fails`                                | drives   ||
| `shell_init_bash_emits_gcd_function`                       | drives   ||
| `shell_init_zsh_emits_gcd_function`                        | drives   ||
| ~~`shell_init_posix_does_not_use_paren_function_syntax`~~  | **dead** | deleted — see *Deletions* |
| `shell_init_posix_unaliases_gcd_first`                     | sentinel | regression: oh-my-zsh's `gcd='git checkout'` shadowed our function definition |
| `shell_init_powershell_unaliases_gcd_first`                | drives   ||
| `shell_init_fish_emits_function_block`                     | drives   ||
| `shell_init_fish_quotes_target_with_double_dash`           | sentinel | regression: fish wildcard expansion mangled `[`, `]`, `*` paths without `cd -- "$target"` |
| `shell_init_powershell_emits_function_block`               | drives   ||
| `shell_init_posix_no_arg_invokes_switch`                   | drives   ||
| `shell_init_fish_no_arg_invokes_switch`                    | drives   ||
| `shell_init_powershell_no_arg_invokes_switch`              | drives   ||
| `shell_init_rejects_unknown_shell`                         | drives   ||
| `list_format_table_is_default`                             | drives   ||

### `tests/config_tests.rs` — 8 tests, **all drives**

| Test                                            | Bucket  |
|:------------------------------------------------|:--------|
| `defaults_are_sane`                             | drives  |
| `placeholders_expand`                           | drives  |
| `placeholders_no_optional_args_leave_repo_only` | drives  |
| `load_returns_defaults_when_no_file`            | drives  |
| `load_parses_repo_config`                       | drives  |
| `write_default_creates_file`                    | drives  |
| `write_default_refuses_overwrite`               | drives  |
| `malformed_config_returns_error`                | drives  |

### `tests/doctor_tests.rs` — 26 tests → 25 after audit

| Test                                                                | Bucket   | Notes |
|:--------------------------------------------------------------------|:---------|:------|
| `fresh_repo_without_config_reports_defaults_assumed`                | drives   ||
| `invalid_toml_marks_config_check_failed_with_severity_failed`       | drives   ||
| `valid_toml_marks_config_check_ok`                                  | drives   ||
| `severity_ok_when_all_checks_ok`                                    | drives   | hand-built report (env-independent) |
| `severity_warning_when_any_check_warns`                             | drives   | hand-built report |
| `severity_failed_dominates_warning`                                 | drives   | hand-built report |
| `dangling_guard_reference_is_failed`                                | drives   ||
| `matching_guard_reference_is_ok`                                    | drives   ||
| `unsupported_when_predicate_is_failed`                              | drives   ||
| `negated_supported_keyword_is_ok`                                   | drives   ||
| `unsupported_keyword_on_rhs_of_and_is_failed`                       | drives   ||
| `file_exists_when_predicate_is_ok`                                  | drives   ||
| `no_when_predicates_is_ok`                                          | drives   ||
| `when_predicates_detail_counts_checked_predicates_not_keywords`     | sentinel | regression: detail used `SUPPORTED_WHEN_PREFIXES.len()` and reported "1 predicate" regardless |
| `when_predicates_detail_says_none_when_no_predicates_configured`    | sentinel | regression: same miscount when no predicates configured |
| `missing_command_binary_is_warning`                                 | drives   ||
| `resolvable_command_binary_is_ok`                                   | drives   ||
| `extract_binary_handles_shell_quoted_run_strings`                   | sentinel | regression: `split_whitespace` returned `"my` for `"my tool" --flag`; shell-words migration |
| `base_dir_existing_and_writable_is_ok`                              | drives   ||
| `base_dir_missing_but_parent_writable_is_ok`                        | drives   ||
| ~~`prunable_check_detail_uses_singular_plural_correctly`~~          | **dead** | deleted — see *Deletions* |
| `fresh_repo_has_no_prunable_worktrees`                              | drives   ||
| `orphan_unmerged_gwm_branch_is_warning`                             | drives   ||
| `merged_gwm_branch_is_not_flagged_as_orphan`                        | drives   ||
| `merged_via_merge_commit_gwm_branch_is_not_flagged_as_orphan`       | drives   ||
| `non_gwm_branch_is_not_flagged_as_orphan`                           | drives   ||

### `tests/flake_tests.rs` — 5 tests, **all drives**

| Test                                                  | Bucket  |
|:------------------------------------------------------|:--------|
| `flake_exists_at_repo_root`                           | drives  |
| `flake_declares_top_level_description_inputs_outputs` | drives  |
| `flake_exposes_gwm_package_via_build_rust_package`    | drives  |
| `flake_exposes_runnable_app`                          | drives  |
| `flake_exposes_dev_shell_with_rust_toolchain`         | drives  |

### `tests/naming_tests.rs` — 10 tests, **all drives**

| Test                                          | Bucket  |
|:----------------------------------------------|:--------|
| `kebab_normalizes`                            | drives  |
| `kebab_treats_punctuation_as_separator`       | drives  |
| `branch_validation`                           | drives  |
| `all_branch_types_accepted`                   | drives  |
| `invalid_issue_must_be_digits`                | drives  |
| `description_normalized_before_validation`    | drives  |
| `parse_roundtrip`                             | drives  |
| `parse_rejects_garbage`                       | drives  |
| `renders_paths`                               | drives  |
| `renders_with_custom_patterns`                | drives  |

### `tests/tui_app_tests.rs` — 56 tests, none deleted

| Test                                                              | Bucket   | Notes |
|:------------------------------------------------------------------|:---------|:------|
| `new_loads_main_worktree`                                          | drives   ||
| `enter_create_initializes_form`                                    | drives   ||
| `create_field_navigation_loops`                                    | drives   ||
| `create_type_navigation_loops`                                     | drives   ||
| `create_push_only_digits_on_issue`                                 | drives   ||
| `create_push_accepts_desc_chars`                                   | drives   ||
| `enter_confirm_delete_refuses_main`                                | drives   ||
| `toggle_delete_branch_flips`                                       | drives   ||
| `next_prev_with_single_entry_stays_put`                            | drives   ||
| `refresh_keeps_selection_in_bounds`                                | drives   ||
| `sidebar_open_by_default`                                          | drives   ||
| `toggle_sidebar_flips_open_flag`                                   | drives   ||
| `toggle_sidebar_when_closed_resets_focus_to_list`                  | drives   ||
| `toggle_focus_only_works_when_sidebar_open`                        | drives   ||
| `first_selects_first_worktree`                                     | drives   ||
| `last_selects_last_worktree`                                       | drives   ||
| `handle_g_motion_tracks_pending_then_jumps_to_first`               | drives   ||
| `pending_g_resets_on_other_key`                                    | drives   ||
| `sidebar_scroll_clamps_to_zero`                                    | drives   ||
| `sidebar_scroll_clamps_at_max`                                     | drives   ||
| `focus_routes_navigation_to_sidebar`                               | drives   ||
| `next_prev_invalidate_sidebar_cache`                               | drives   ||
| `refresh_invalidates_sidebar_cache`                                | drives   ||
| `filter_state_defaults_to_inactive_and_empty`                      | drives   ||
| `enter_filter_activates_capture_and_disarms_gg`                    | drives   ||
| `enter_filter_drops_sidebar_focus`                                 | sentinel | regression: PR #44 Copilot review — sidebar focus survived `/` and broke j/k after Enter |
| `enter_filter_preserves_existing_query`                            | drives   ||
| `filter_push_char_appends_to_query`                                | drives   ||
| `filter_pop_char_removes_last_char`                                | drives   ||
| `filter_pop_char_on_empty_is_noop`                                 | drives   ||
| `exit_filter_keep_disables_capture_keeps_query`                    | drives   ||
| `exit_filter_cancel_clears_query`                                  | drives   ||
| `filtered_indices_returns_all_when_query_empty`                    | drives   ||
| `filtered_indices_keeps_only_matching_worktrees`                   | drives   ||
| `filtered_indices_supports_subsequence_match`                      | drives   | issue #21 contract |
| `filtered_indices_ranks_substring_above_subsequence`               | drives   | issue #21 ranking |
| `filtered_indices_skips_when_no_match`                             | drives   ||
| `selected_returns_filtered_worktree`                               | drives   ||
| `selected_returns_none_when_filter_matches_nothing`                | drives   ||
| `next_navigates_within_filtered_subset_and_wraps`                  | drives   ||
| `prev_navigates_within_filtered_subset_and_wraps`                  | drives   ||
| `first_and_last_jump_inside_filtered_subset`                       | drives   ||
| `filter_push_clamps_selection_when_subset_shrinks`                 | drives   ||
| `exit_filter_cancel_restores_full_list_selection`                  | drives   ||
| `picker_mode_defaults_to_false`                                    | drives   ||
| `new_picker_at_enables_picker_mode`                                | drives   ||
| `new_picker_at_opens_filter_bar`                                   | drives   | issue #22 contract |
| `picker_confirm_records_selected_path`                             | drives   ||
| `picker_confirm_with_no_selection_keeps_result_none`               | drives   ||
| `picker_confirm_outside_picker_mode_is_inert`                      | drives   ||
| `picker_should_exit_defaults_false`                                | drives   ||
| `picker_confirm_with_selection_signals_exit`                       | drives   ||
| `picker_confirm_without_selection_does_not_signal_exit`            | sentinel | regression: PR #53 Copilot review — empty filter result triggered exit-1 |
| `picker_confirm_without_selection_reports_status`                  | sentinel | regression: PR #53 — empty picker result was silently swallowed without status hint |
| `picker_cancel_signals_exit_without_path`                          | sentinel | regression: PR #53 — picker footer reads `esc:cancel` but Esc-in-filter only cleared the query |
| `picker_cancel_outside_picker_mode_is_inert`                       | drives   ||

### `tests/worktree_integration.rs` — 16 tests, **all drives**

| Test                                                       | Bucket  |
|:-----------------------------------------------------------|:--------|
| `discover_finds_repo`                                      | drives  |
| `list_includes_main_worktree`                              | drives  |
| `add_creates_branch_and_worktree`                          | drives  |
| `add_refuses_to_clobber_existing_dir`                      | drives  |
| `remove_deletes_dir_and_prunes`                            | drives  |
| `remove_with_delete_branch_drops_branch`                   | drives  |
| `find_fuzzy_matches_substring`                             | drives  |
| `find_fuzzy_errors_on_ambiguous`                           | drives  |
| `prune_returns_zero_when_clean`                            | drives  |
| `repo_name_derives_from_workdir`                           | drives  |
| `discover_from_inside_linked_worktree_walks_back_to_main`  | drives  |
| `git_log_oneline_returns_seed_commit`                      | drives  |
| `git_log_oneline_respects_limit`                           | drives  |
| `git_status_short_empty_on_clean_repo`                     | drives  |
| `git_status_short_lists_untracked_file`                    | drives  |
| `git_log_oneline_errors_outside_repo`                      | drives  |

## Reading guide for future PRs

When adding a test, ask yourself which bucket it falls into **before**
committing it:

- If it exercises a production code path: bucket #1. No prefix needed.
- If it guards a specific incident (regression, review comment, escape
  hatch): bucket #2. Prefix the test body with `// regression: <one-line>`.
  The `<one-line>` should name the incident (PR number, issue, or a
  short symptom) — enough that a future reader can find the context
  without `git blame`.
- If you cannot say which production path it pins, or if another test
  already covers that path: it is bucket #3. Don't write it.

Goal: signal-to-noise per test stays high even as the suite grows.
A 250-test suite where 50 are noise serves the project worse than
the 190-test suite this audit leaves behind.
