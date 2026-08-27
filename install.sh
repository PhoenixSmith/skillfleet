#!/usr/bin/env sh
#
# Skillfleet installer.
#
#   curl -fsSL https://raw.githubusercontent.com/PhoenixSmith/skillfleet/main/install.sh | sh
#
# Downloads the latest release asset matching the current platform, verifies
# the tarball, and installs `skillfleet` and `skillfleet-tui` into
# ${SKILLFLEET_BINDIR:-~/.local/bin}.
#
# Overrides:
#   SKILLFLEET_VERSION  install a specific version instead of latest (e.g. 0.2.0)
#   SKILLFLEET_BINDIR   install directory instead of ~/.local/bin
#   SKILLFLEET_REPO     repository, default PhoenixSmith/skillfleet

set -eu

REPO="${SKILLFLEET_REPO:-PhoenixSmith/skillfleet}"
BINDIR="${SKILLFLEET_BINDIR:-"$HOME/.local/bin"}"
API="https://api.github.com/repos/${REPO}/releases"

log() { printf '%s\n' "$*"; }
die() { log "skillfleet: error: $*" >&2; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || die "missing required tool: $1"; }
need curl
need tar

case "$(uname -s)" in
  Linux)  OS=linux ;;
  Darwin) OS=darwin ;;
  *)      die "unsupported OS: $(uname -s)" ;;
esac
case "$(uname -m)" in
  x86_64|amd64)  ARCH=amd64 ;;
  aarch64|arm64) ARCH=arm64 ;;
  *)             die "unsupported architecture: $(uname -m)" ;;
esac

if [ -n "${SKILLFLEET_VERSION:-}" ]; then
  VERSION="$SKILLFLEET_VERSION"
else
  log "resolving latest release..."
  VERSION="$(
    curl -fsSL -H "Accept: application/vnd.github+json" \
      "${API}/latest" |
      sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' |
      sed 's/^v//'
  )"
  [ -n "$VERSION" ] || die "could not resolve latest version from ${API}/latest"
fi

ASSET="skillfleet-${VERSION}-${OS}-${ARCH}.tar.gz"
URL="https://github.com/${REPO}/releases/download/v${VERSION}/${ASSET}"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT INT TERM

log "downloading ${ASSET}"
curl -fsSL -o "${TMP}/${ASSET}" "$URL" || die "download failed: $URL"

log "verifying tarball"
tar -tzf "${TMP}/${ASSET}" >/dev/null || die "corrupt archive"
tar -tzf "${TMP}/${ASSET}" | grep -q 'skillfleet' || die "archive does not contain skillfleet"

install -d "$BINDIR"
tar -xzf "${TMP}/${ASSET}" -C "$TMP"
install -m 0755 "${TMP}/skillfleet" "${BINDIR}/skillfleet"
install -m 0755 "${TMP}/skillfleet-tui" "${BINDIR}/skillfleet-tui"

log ""
log "installed skillfleet ${VERSION} to ${BINDIR}"
log "  skillfleet      (CLI)"
log "  skillfleet-tui  (interactive inspector)"
log ""
case ":${PATH}:" in
  *":${BINDIR}:"*) : ;;
  *) log "note: ${BINDIR} is not on your PATH. Add it, e.g. in ~/.bashrc:" ;;
esac
log "run 'skillfleet --help' to get started."