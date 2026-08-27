#!/usr/bin/env bash
# IRL live-agent test: fresh OpenClaw install consumes skillfleet-managed skills.
# Requires the official OpenClaw installer (fetched from openclaw.ai/install.sh
# unless OPENCLAW_INSTALL_URL is set). Same flow as Hermes: fresh install,
# co-located config, routing, native-skill vacuum migration, then confirms
# OpenClaw's skills dir is populated with symlinks.
set -euo pipefail
source "$(dirname "$0")/common.sh"
export HOME="${HOME:-/root}"
export DEBIAN_FRONTEND=noninteractive

echo "=== IRL: live OpenClaw ==="
find_skillfleet

if ! command -v openclaw >/dev/null 2>&1; then
  apt-get update -qq >/dev/null 2>&1 || true
  apt-get install -y -qq git curl ca-certificates >/dev/null 2>&1 || true
  INSTALLER="${OPENCLAW_INSTALL_URL:-https://openclaw.ai/install.sh}"
  curl -fsSL "$INSTALLER" -o /tmp/ocl-install.sh
  bash /tmp/ocl-install.sh --no-onboard 2>&1 | tail -6 || warn "openclaw installer returned nonzero"
  export PATH="$HOME/.openclaw/bin:$PATH"
fi
command -v openclaw >/dev/null 2>&1 && pass "openclaw installed ($(openclaw --version 2>&1 | head -1))" || fail "openclaw not installed"

EP="$HOME/.openclaw/skills"
LIB=/tmp/lib
mkdir -p "$LIB" "$EP"
cd "$LIB"
skillfleet init --library "$LIB" >/dev/null
[ -f "$LIB/skillfleet.toml" ] && pass "manifest co-located" || fail "manifest missing"
export SKILLFLEET_CONFIG="$LIB/skillfleet.toml"

write_skill "$LIB/skills" myskill
write_skill "$LIB/skills" demo
skillfleet endpoint add openclaw "$EP"
skillfleet skill add myskill --source skills/myskill --to openclaw
skillfleet skill add demo    --source skills/demo    --to openclaw
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

# Live OpenClaw skills dir.
echo "--- openclaw skills dir ---"
ls -la "$EP/"
echo "--- openclaw skills list (skips bundled) ---"
openclaw skills list 2>&1 | head -8 || warn "openclaw skills list unavailable"

echo "=== IRL LIVE OPENCLAW COMPLETE ==="