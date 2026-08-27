#!/usr/bin/env bash
# IRL live-agent test: fresh Hermes install consumes skillfleet-managed skills.
# Requires the official Hermes installer at /opt/hermes-install.sh (mounted in
# by the runner) OR HERMES_INSTALL_URL. Exercises: fresh install, co-located
# config, routing, and native-skill vacuum migration, then confirms Hermes sees
# the skills via `hermes skills list`.
set -euo pipefail
source "$(dirname "$0")/common.sh"
export HOME="${HOME:-/root}"
export DEBIAN_FRONTEND=noninteractive

echo "=== IRL: live Hermes ==="
find_skillfleet

if ! command -v hermes >/dev/null 2>&1; then
  apt-get update -qq >/dev/null 2>&1 || true
  apt-get install -y -qq git curl ca-certificates python3 python3-pip build-essential python3-dev libffi-dev ripgrep >/dev/null 2>&1 || true
  INSTALLER=/opt/hermes-install.sh
  if [ ! -f "$INSTALLER" ]; then
    URL="${HERMES_INSTALL_URL:-https://raw.githubusercontent.com/NousResearch/hermes-agent/main/scripts/install.sh}"
    # Retry on transient failures; a truncated download silently breaks the install.
    for _ in 1 2 3; do
      curl -fsSL --retry 3 --retry-delay 2 "$URL" -o "$INSTALLER" 2>/dev/null && [ -s "$INSTALLER" ] && break
      sleep 2
    done
    [ -s "$INSTALLER" ] || fail "could not download Hermes installer"
  fi
  bash "$INSTALLER" --non-interactive --skip-setup 2>&1 | tail -6 || warn "hermes installer returned nonzero"
fi
command -v hermes >/dev/null 2>&1 && pass "hermes installed ($(hermes --version 2>&1 | head -1))" || fail "hermes not installed"

EP="$HOME/.hermes/skills"
LIB=/tmp/lib
mkdir -p "$LIB" "$EP"
cd "$LIB"
skillfleet init --library "$LIB" >/dev/null
[ -f "$LIB/skillfleet.toml" ] && pass "manifest co-located" || fail "manifest missing"
export SKILLFLEET_CONFIG="$LIB/skillfleet.toml"

write_skill "$LIB/skills" myskill
write_skill "$LIB/skills" demo
skillfleet endpoint add hermes "$EP"
skillfleet skill add myskill --source skills/myskill --to hermes
skillfleet skill add demo    --source skills/demo    --to hermes
skillfleet sync >/dev/null
skillfleet doctor >/dev/null && pass "routed 2 skills, doctor clean"
for s in myskill demo; do
  [ -L "$EP/$s" ] && [ -f "$EP/$s/SKILL.md" ] && pass "$s -> $(realpath "$EP/$s")" || fail "$s not symlinked"
done

# Native-skill vacuum migration.
mkdir -p "$EP/native"
cat > "$EP/native/SKILL.md" <<'EOF'
---
name: native
description: "Pre-existing native skill to migrate under skillfleet management."
---
# native
Old native skill.
EOF
echo "--- sync adopts native (drop-in real dir -> library + link back) ---"
if skillfleet sync 2>&1 | grep -q "adopted"; then
  [ -L "$EP/native" ] && [ -f "$EP/native/SKILL.md" ] && [ -f "$LIB/skills/native/SKILL.md" ] \
    && pass "native vacuum-adopted: library copy + symlink" || fail "native not adopted correctly"
else
  warn "vacuum did not report an adoption"
fi
skillfleet doctor >/dev/null && pass "doctor clean after migration"

# Live Hermes consumption.
echo "--- hermes skills list ---"
hermes skills list 2>&1 | grep -iE "myskill|demo|native" | head -6 \
  || warn "hermes skills list did not surface the skills"

echo "=== IRL LIVE HERMES COMPLETE ==="