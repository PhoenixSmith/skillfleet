#!/usr/bin/env bash
# IRL suite runner — spins up ephemeral containers and runs the live-agent
# tests against the freshly built binaries. Real end-to-end verification with
# a live Hermes and/or OpenClaw, not mocks.
#
#   tests/irl/run.sh            # full suite (CLI smoke + Hermes + OpenClaw)
#   tests/irl/run.sh --cli      # CLI-only smoke (fast, no agent install)
#   tests/irl/run.sh --hermes   # Hermes live only
#   tests/irl/run.sh --openclaw # OpenClaw live only
#
# Requires: docker + network. Builds release binaries first via `make build`.

set -euo pipefail
cd "$(dirname "$0")/../.."          # repo root

RUN_CLI=1 RUN_HERMES=1 RUN_OPENCLAW=1
for arg in "$@"; do
  case "$arg" in
    --cli) RUN_HERMES=0; RUN_OPENCLAW=0 ;;
    --hermes) RUN_CLI=0; RUN_OPENCLAW=0 ;;
    --openclaw) RUN_CLI=0; RUN_HERMES=0 ;;
    *) echo "unknown arg: $arg"; exit 2 ;;
  esac
done

command -v docker >/dev/null || { echo "IRL suite requires docker"; exit 1; }

echo "==> building release binaries"
make build >/dev/null 2>&1 || { echo "build failed"; exit 1; }

STAGE="$(mktemp -d)"
mkdir -p "$STAGE/bin" "$STAGE/scripts"
cp target/release/skillfleet target/release/skillfleet-tui "$STAGE/bin/"
cp tests/irl/common.sh tests/irl/01-cli-smoke.sh "$STAGE/scripts/"
cp tests/irl/02-hermes-live.sh "$STAGE/scripts/"
cp tests/irl/03-openclaw-live.sh "$STAGE/scripts/"
chmod +x "$STAGE"/bin/* "$STAGE"/scripts/*.sh

run_in() { # $1 image, $2 script, $3 extra mounts (optional)
  local img="$1" script="$2" tag
  local extra="${3:-}"
  tag="sf-irl-$(basename "$script" .sh)-$$"
  echo "=== $script ==="
  docker rm -f "$tag" >/dev/null 2>&1 || true
  # shellcheck disable=SC2086
  docker run --rm --name "$tag" \
    -v "$STAGE/bin:/opt/bin" \
    -v "$STAGE/scripts:/opt/scripts" \
    -e HERMES_INSTALL_URL="https://raw.githubusercontent.com/NousResearch/hermes-agent/main/scripts/install.sh" \
    -e OPENCLAW_INSTALL_URL="https://openclaw.ai/install.sh" \
    $extra "$img" /bin/bash "/opt/scripts/$(basename "$script")" \
    && echo "SUCCESS: $script" || { echo "FAILED: $script"; RC=1; }
}

RC=0
[ "$RUN_CLI" = 1 ] && run_in debian:12-slim "$STAGE/scripts/01-cli-smoke.sh" ""
[ "$RUN_HERMES" = 1 ] && run_in debian:12 "$STAGE/scripts/02-hermes-live.sh" ""
[ "$RUN_OPENCLAW" = 1 ] && run_in debian:12 "$STAGE/scripts/03-openclaw-live.sh" ""

rm -rf "$STAGE"
echo ""
[ "$RC" = 0 ] && echo "IRL SUITE ALL GREEN" || echo "IRL SUITE FAILED"
exit "$RC"