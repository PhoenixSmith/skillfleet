#!/usr/bin/env bash
# Shared helpers for the IRL (in-real-life) live-agent verification suite.
# Sources: tests/irl/*.sh must `source "$(dirname "$0")/common.sh"`.
# These scripts run INSIDE a fresh ephemeral container against a live agent,
# so they assume: bash, curl, a root-ish HOME, and an empty skillfleet state.

set -euo pipefail

log()  { printf '%s\n' "$*"; }
pass() { printf 'PASS: %s\n' "$*"; }
fail() { printf 'FAIL: %s\n' "$*"; exit 1; }
warn() { printf 'WARN: %s\n' "$*"; }

# Resolve skillfleet binaries mounted at /opt/bin (build tree) or on PATH.
find_skillfleet() {
  if [ -x /opt/bin/skillfleet ]; then
    cp /opt/bin/skillfleet /opt/bin/skillfleet-tui /usr/local/bin/
    chmod +x /usr/local/bin/skillfleet /usr/local/bin/skillfleet-tui
  fi
  command -v skillfleet >/dev/null || fail "skillfleet not found; mount build bin/ at /opt/bin or add to PATH"
  command -v skillfleet-tui >/dev/null || warn "skillfleet-tui not found (TUI check optional)"
}

# Print a valid SKILL.md for skill NAME to a <name>/SKILL.md under $1.
write_skill() { # $1 dir root, $2 name
  local dir="$1" name="$2"
  mkdir -p "$dir/$name"
  cat > "$dir/$name/SKILL.md" <<EOF
---
name: $name
description: "Managed by skillfleet for the IRL live-agent verification."
---
# $name
$name ping from the IRL harness.
EOF
}