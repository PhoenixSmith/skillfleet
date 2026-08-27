#!/usr/bin/env bash
# CLI-only IRL smoke test: fresh skillfleet install behavior with out any agent.
# Verifies the repo-local manifest default, init co-location + export hint, and
# the env-var/upward-walk resolution matrix. Run inside a clean container.
set -euo pipefail
source "$(dirname "$0")/common.sh"
export HOME="${HOME:-/root}"

echo "=== IRL: CLI-only smoke (no agent) ==="
find_skillfleet
[ -z "${SKILLFLEET_CONFIG:-}" ] || { fail "SKILLFLEET_CONFIG should be unset for a fresh test"; }
[ ! -e "$HOME/.config/skillfleet" ] && pass "no pre-existing config dir in clean env" || warn "pre-existing config dir present"

mkdir -p /tmp/lib
cd /tmp
INIT=$(skillfleet init --library /tmp/lib)
echo "$INIT"
[ -f /tmp/lib/skillfleet.toml ] && pass "config co-located at library root" || fail "manifest not co-located"
echo "$INIT" | grep -q "export SKILLFLEET_CONFIG=/tmp/lib/skillfleet.toml" \
  && pass "init printed export hint" || fail "no export hint from init"

# Full upsert -> plan -> sync -> doctor happiness
mkdir -p /tmp/ep
skillfleet --config /tmp/lib/skillfleet.toml endpoint add agent /tmp/ep
write_skill /tmp/lib/skills demo
skillfleet --config /tmp/lib/skillfleet.toml skill add demo --source skills/demo --to agent
skillfleet --config /tmp/lib/skillfleet.toml plan >/dev/null
skillfleet --config /tmp/lib/skillfleet.toml sync >/dev/null
skillfleet --config /tmp/lib/skillfleet.toml doctor >/dev/null && pass "upsert->plan->sync->doctor clean"
[ -L /tmp/ep/demo ] && [ -f /tmp/ep/demo/SKILL.md ] && pass "demo routed as symlink" || fail "demo not symlinked"

# Upward-walk from a nested cwd with NO env var finds the repo-local config.
cd /tmp/lib/skills/demo
unset SKILLFLEET_CONFIG
skillfleet status 2>&1 | grep -q "health: ok" && pass "upward-walk found config from nested cwd (no env)" || fail "upward-walk failed"

# Unrelated cwd, no env -> clean failure (no silent wrong config).
cd /tmp
if skillfleet status >/dev/null 2>&1; then
  warn "status succeeded from unrelated cwd (unexpected fallback)"
else
  pass "clean failure from unrelated cwd (no manifest, no env)"
fi

# Env var set -> location-independent.
export SKILLFLEET_CONFIG=/tmp/lib/skillfleet.toml
cd /tmp
skillfleet status 2>&1 | grep -q "health: ok" && pass "env var makes it location-independent" || fail "env override failed"

echo "=== IRL CLI SMOKE COMPLETE ==="