#!/usr/bin/env bash
# Common functions and variables for all scripts

# ── Speckit Config ──────────────────────────────────────────────
# Defaults (overridden by .specify/speckit.env if present)
SPECKIT_SPEC_DIR=".specify/specs"
SPECKIT_SCRIPTS_DIR=".specify/scripts/bash"
SPECKIT_TEMPLATES_DIR=".specify/templates"
SPECKIT_CONSTITUTION=".specify/memory/constitution.md"
SPECKIT_BRANCH_PATTERN="feat/#{number}-{slug}"
SPECKIT_BRANCH_PREFIX="feat/#"
SPECKIT_BRANCH_SEPARATOR="-"
SPECKIT_BRANCH_REGEX="^[a-z]+/#[0-9]+-|^[0-9]{3}-"
SPECKIT_SPEC_PATTERN="{number}-{slug}"
SPECKIT_NUMBER_PADDING=0
SPECKIT_PREFIX_REGEX_STANDARD="^([0-9]+)-"
SPECKIT_PREFIX_REGEX_PREFIXED="^[a-z]+/#([0-9]+)-"

# Load project config if available (call after get_repo_root)
load_speckit_config() {
    local repo_root="$1"
    local config_file="$repo_root/.specify/speckit.env"
    if [[ -f "$config_file" ]]; then
        # Source the config file (bash-compatible key=value format)
        # shellcheck disable=SC1090
        source "$config_file"
    fi
}

# Get repository root, with fallback for non-git repositories
get_repo_root() {
    if git rev-parse --show-toplevel >/dev/null 2>&1; then
        git rev-parse --show-toplevel
    else
        # Fall back to script location for non-git repos
        local script_dir="$(CDPATH="" cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
        (cd "$script_dir/../../.." && pwd)
    fi
}

# Get current branch, with fallback for non-git repositories
get_current_branch() {
    # First check if SPECIFY_FEATURE environment variable is set
    if [[ -n "${SPECIFY_FEATURE:-}" ]]; then
        echo "$SPECIFY_FEATURE"
        return
    fi

    # Then check git if available
    if git rev-parse --abbrev-ref HEAD >/dev/null 2>&1; then
        git rev-parse --abbrev-ref HEAD
        return
    fi

    # For non-git repos, try to find the latest feature directory
    local repo_root=$(get_repo_root)
    load_speckit_config "$repo_root"
    local specs_dir="$repo_root/$SPECKIT_SPEC_DIR"

    if [[ -d "$specs_dir" ]]; then
        local latest_feature=""
        local highest=0

        for dir in "$specs_dir"/*; do
            if [[ -d "$dir" ]]; then
                local dirname=$(basename "$dir")
                if [[ "$dirname" =~ $SPECKIT_PREFIX_REGEX_STANDARD ]]; then
                    local number=${BASH_REMATCH[1]}
                    number=$((10#$number))
                    if [[ "$number" -gt "$highest" ]]; then
                        highest=$number
                        latest_feature=$dirname
                    fi
                fi
            fi
        done

        if [[ -n "$latest_feature" ]]; then
            echo "$latest_feature"
            return
        fi
    fi

    echo "main"  # Final fallback
}

# Check if we have git available
has_git() {
    git rev-parse --show-toplevel >/dev/null 2>&1
}

check_feature_branch() {
    local branch="$1"
    local has_git_repo="$2"

    # For non-git repos, we can't enforce branch naming but still provide output
    if [[ "$has_git_repo" != "true" ]]; then
        echo "[specify] Warning: Git repository not detected; skipped branch validation" >&2
        return 0
    fi

    # Validate branch against configured pattern (SPECKIT_BRANCH_REGEX)
    # Supports pipe-separated alternates, e.g. "^[a-z]+/#[0-9]+-|^[0-9]{3}-"
    if [[ ! "$branch" =~ $SPECKIT_BRANCH_REGEX ]]; then
        echo "ERROR: Not on a feature branch. Current branch: $branch" >&2
        echo "Feature branches should match pattern: $SPECKIT_BRANCH_REGEX" >&2
        echo "Example: $SPECKIT_BRANCH_PREFIX<number>$SPECKIT_BRANCH_SEPARATOR<slug>" >&2
        return 1
    fi

    return 0
}

get_feature_dir() { echo "$1/$SPECKIT_SPEC_DIR/$2"; }

# Find feature directory by numeric prefix instead of exact branch match
# This allows multiple branches to work on the same spec (e.g., 004-fix-bug, 004-add-feature)
find_feature_dir_by_prefix() {
    local repo_root="$1"
    local branch_name="$2"
    local specs_dir="$repo_root/$SPECKIT_SPEC_DIR"

    # Extract numeric prefix from branch using configured regex patterns
    local prefix=""
    if [[ "$branch_name" =~ $SPECKIT_PREFIX_REGEX_STANDARD ]]; then
        prefix="${BASH_REMATCH[1]}"
    elif [[ "$branch_name" =~ $SPECKIT_PREFIX_REGEX_PREFIXED ]]; then
        prefix="${BASH_REMATCH[1]}"
    else
        # If branch doesn't have numeric prefix, fall back to exact match
        echo "$specs_dir/$branch_name"
        return
    fi

    # Search for directories in specs/ that match this prefix
    # Support both "580-feature" and "#580-feature" patterns
    local matches=()
    if [[ -d "$specs_dir" ]]; then
        # Try standard format first (580-feature)
        for dir in "$specs_dir"/"$prefix"-*; do
            if [[ -d "$dir" ]]; then
                matches+=("$(basename "$dir")")
            fi
        done
        # Try with hash prefix (#580-feature)
        for dir in "$specs_dir"/*"$prefix"-*; do
            if [[ -d "$dir" ]]; then
                local basename_dir=$(basename "$dir")
                # Avoid duplicates
                if [[ ! " ${matches[*]} " =~ " ${basename_dir} " ]]; then
                    matches+=("$basename_dir")
                fi
            fi
        done
    fi

    # Fallback: if no numeric match, try matching by slug (part after number-separator)
    if [[ ${#matches[@]} -eq 0 && -d "$specs_dir" ]]; then
        # Extract slug from branch: "feat/#000-acs-sub" -> "acs-sub", "004-feature" -> "feature"
        local slug=""
        if [[ "$branch_name" =~ $SPECKIT_PREFIX_REGEX_PREFIXED ]]; then
            slug="${branch_name#*"${SPECKIT_BRANCH_SEPARATOR}"}"
            # Remove everything up to and including the first separator after the prefix
            slug=$(echo "$branch_name" | sed "s/^[a-z]*\/#[0-9]*${SPECKIT_BRANCH_SEPARATOR}//")
        elif [[ "$branch_name" =~ $SPECKIT_PREFIX_REGEX_STANDARD ]]; then
            slug="${branch_name#*-}"
        fi
        if [[ -n "$slug" && -d "$specs_dir/$slug" ]]; then
            matches+=("$slug")
        fi
    fi

    # Handle results
    if [[ ${#matches[@]} -eq 0 ]]; then
        # No match found - return the branch name path (will fail later with clear error)
        echo "$specs_dir/$branch_name"
    elif [[ ${#matches[@]} -eq 1 ]]; then
        # Exactly one match - perfect!
        echo "$specs_dir/${matches[0]}"
    else
        # Multiple matches - this shouldn't happen with proper naming convention
        echo "ERROR: Multiple spec directories found with prefix '$prefix': ${matches[*]}" >&2
        echo "Please ensure only one spec directory exists per numeric prefix." >&2
        echo "$specs_dir/$branch_name"  # Return something to avoid breaking the script
    fi
}

get_feature_paths() {
    local repo_root=$(get_repo_root)

    # Load config early so all downstream functions use configured values
    load_speckit_config "$repo_root"

    local current_branch=$(get_current_branch)
    local has_git_repo="false"

    if has_git; then
        has_git_repo="true"
    fi

    # Resolve the feature directory. Priority:
    #   1. Prefix-based lookup from the current branch. For issue-first numbering the
    #      branch always encodes the feature number (e.g. feat/#42-… → 42-…), so the
    #      branch reflects where you ACTUALLY are — it beats a possibly-stale pointer.
    #   2. Explicit .specify/feature.json pointer, used only when the branch yields no
    #      existing spec dir (branch with no numeric prefix, or standalone specify).
    #      feature.json is a per-checkout, git-ignored runtime pointer.
    local feature_dir
    feature_dir=$(find_feature_dir_by_prefix "$repo_root" "$current_branch")
    if [[ ! -d "$feature_dir" ]]; then
        local feature_json="$repo_root/.specify/feature.json"
        if [[ -f "$feature_json" ]]; then
            local fj_rel
            fj_rel=$(grep -o '"feature_directory"[[:space:]]*:[[:space:]]*"[^"]*"' "$feature_json" 2>/dev/null \
                | sed 's/.*"feature_directory"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/')
            if [[ -n "$fj_rel" && -d "$repo_root/$fj_rel" ]]; then
                feature_dir="$repo_root/$fj_rel"
            fi
        fi
    fi

    cat <<EOF
REPO_ROOT='$repo_root'
CURRENT_BRANCH='$current_branch'
HAS_GIT='$has_git_repo'
FEATURE_DIR='$feature_dir'
FEATURE_SPEC='$feature_dir/spec.md'
IMPL_PLAN='$feature_dir/plan.md'
TASKS='$feature_dir/tasks.md'
RESEARCH='$feature_dir/research.md'
DATA_MODEL='$feature_dir/data-model.md'
QUICKSTART='$feature_dir/quickstart.md'
CONTRACTS_DIR='$feature_dir/contracts'
EOF
}

check_file() { [[ -f "$1" ]] && echo "  ✓ $2" || echo "  ✗ $2"; }
check_dir() { [[ -d "$1" && -n $(ls -A "$1" 2>/dev/null) ]] && echo "  ✓ $2" || echo "  ✗ $2"; }
