<div align="center">
  <img src="assets/logo.svg" alt="Skillfleet logo" width="128" height="128">

  <h1>Skillfleet</h1>

  <p><strong>Git-backed skill distribution for AI agents.</strong><br>
  One canonical library. Arbitrary named endpoints. Explicit symlink routing.</p>

  <p>
    <a href="https://github.com/PhoenixSmith/skillfleet"><img src="https://img.shields.io/badge/version-0.2.0-f59e0b" alt="Version 0.2.0"></a>
    <img src="https://img.shields.io/badge/CLI-Rust-dea584?logo=rust&logoColor=white" alt="Rust CLI">
    <img src="https://img.shields.io/badge/TUI-Go%20%2B%20Bubble%20Tea-00ADD8?logo=go&logoColor=white" alt="Go TUI">
    <img src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS-4c9aff" alt="Linux and macOS">
    <img src="https://img.shields.io/badge/license-MIT-34d399" alt="MIT license">
  </p>
</div>

Skillfleet keeps skills in one canonical library and routes whole skill directories into arbitrary **named endpoints** using symlinks. It does not hardcode Hermes, OpenClaw, Claude Code, Codex, Pi, or any other runtime. If an agent reads skills from a directory, name that directory and target it.

```bash
skillfleet endpoint add claude ~/.claude/skills     # name a directory an agent reads
skillfleet skill add cleanup --source skills/cleanup --to claude codex hermes
skillfleet plan && skillfleet sync                  # inspect, then link
```

## Why

Without a manager, multi-agent skill setups become a thicket of copied directories, consumer-to-consumer symlinks, stale third-party snapshots, and edits landing in the wrong place.

Skillfleet provides:

- **One commit-able source of truth** — edit a skill once, every endpoint sees it;
- **Arbitrary named endpoints** — no baked-in runtime compatibility table;
- **Explicit per-skill routing** — each skill goes exactly where you send it;
- **Owned/personal skills** alongside **Git-sourced third-party skills** with `update`;
- **Per-endpoint source variants** — different instructions per harness, same skill name;
- **Dry inspection** with `plan` — no writes until you say `sync`;
- **Machine-readable `--json` output** — built for agents and CI, not just humans;
- **Safe conflict handling** — real directories are backed up, never silently deleted.

## Install

Prebuilt binaries for Linux (amd64/arm64) and macOS (amd64/arm64) are
published with each release. Install both the CLI and the interactive TUI with
one command:

```bash
curl -fsSL https://raw.githubusercontent.com/PhoenixSmith/skillfleet/main/install.sh | sh
```

This downloads the release tarball matching your OS + architecture, verifies it,
and installs `skillfleet` and `skillfleet-tui` into `~/.local/bin` (override
with `SKILLFLEET_BINDIR`). Cache the script and review it before running if you
prefer.

For development or packaging from source (requires Rust and Go 1.18+):

```bash
make build                           # target/release/skillfleet{,-tui}
make test
make install PREFIX=/usr/local       # defaults to ~/.local/bin
```

`cargo install --path .` still installs the original CLI by itself. In that case,
build/install `skillfleet-tui` separately, or set `SKILLFLEET_TUI` to its path.

## Binaries & updates

Each release carries a tarball per platform:

- Linux `amd64` / `arm64`
- macOS `amd64` (Intel) / `arm64` (Apple Silicon)

Keep the binaries current:

```bash
skillfleet self update              # fetch latest release and reinstall in place
skillfleet self update --check      # report the newest version without installing
```

`self update` compares the installed version against the latest GitHub release,
downloads the matching-platform tarball, and atomically replaces both binaries
beside the running executable. It requires `curl` and `tar`. There is no
telemetry or background phone-home; checks run only when you invoke them.

## Interactive TUI

After `skillfleet init`, open the routing inspector with:

```bash
skillfleet tui
skillfleet --config ./skillfleet.toml tui
```

The responsive TUI reads the same TOML and live symlink state as the CLI. Changes are staged in memory and the persistent staged counter makes it clear that the filesystem has not changed yet.

- `Tab` / `Shift+Tab`: switch **Skills**, **Endpoints**, and **Plan** views
- `↑` / `↓`: move; `←` / `→`: select an endpoint in the Skills matrix
- `Space`: toggle the focused skill/endpoint route
- `n`: stage a local skill; `a` / `e` / `d`: add, edit, or dependency-safely remove an endpoint
- `Ctrl+S`: review grouped creates/removes/conflicts, then apply; `Esc`: go back
- On each conflict: `s` skip, `k` keep existing, or `b` backup and link; unresolved conflicts block apply
- `/`: search skills; `r`: reload; `?`: contextual help; `q`: quit (with a dirty warning)

Endpoint forms expand paths and validate existence, writability, duplicates, nesting, and library overlap, then report unmanaged entries. Apply uses the existing Rust CLI, runs a safe sync by default, and automatically runs `doctor`; the complete apply/doctor summary is shown afterward. Backup-and-link is available only as an explicit per-conflict choice. The launcher locates `skillfleet-tui` beside `skillfleet`, passes the resolved config path, and points mutations back at the exact CLI executable.

## Quick start

```bash
skillfleet init --library ~/agent-skills

skillfleet endpoint add hermes ~/.hermes/skills
skillfleet endpoint add openclaw ~/.openclaw/skills
skillfleet endpoint add pi ~/.pi/agent/skills
skillfleet endpoint add codex ~/.codex/skills
skillfleet endpoint add claude ~/.claude/skills

# Teach selected agents how to operate Skillfleet safely.
skillfleet self install --to hermes pi codex claude
skillfleet sync

skillfleet skill add cleanup \
  --source skills/cleanup \
  --to hermes openclaw pi codex claude

skillfleet plan
skillfleet sync
skillfleet doctor
```

Endpoint names are labels, not a baked-in compatibility table:

```bash
skillfleet endpoint add future-agent ~/.future-agent/skills
skillfleet skill route cleanup --to hermes future-agent
skillfleet sync
```

## Third-party Git skills

The remote may be one skill at repository root:

```bash
skillfleet skill add upstream-review \
  --git https://github.com/example/review-skill.git \
  --to claude codex
skillfleet update upstream-review
skillfleet sync
```

Or one skill inside a larger repository:

```bash
skillfleet skill add autoreview \
  --git https://github.com/openclaw/agent-skills.git \
  --subdir skills/autoreview \
  --to hermes openclaw
skillfleet update autoreview
```

`update` vendors the selected directory under `<library>/vendor/<name>`. Commit that directory in the library repo to pin and review upstream changes.

Run all subscribed updates from cron/CI:

```bash
skillfleet update && skillfleet sync && skillfleet doctor
```

## Per-endpoint variants

A skill may need different instructions for different harnesses:

```bash
skillfleet skill add impeccable \
  --source skills/impeccable/agents \
  --to hermes claude
skillfleet skill source impeccable \
  --for claude skills/impeccable/claude
```

The endpoint remains generic. Only that route gets the override.

## Agent and CI usage

Commands are non-interactive. Global `--json` emits exactly one document: success is `{ "schema_version": 1, "ok": true, "command": "...", "data": ... }`; failure is `{ "schema_version": 1, "ok": false, "error": { "code": "...", "message": "..." } }`. Exit codes are 0 for success, 1 for operation/verification failure, and 2 for invalid usage. Stable error codes are `usage_error`, `already_exists`, `not_found`, `unknown_endpoint`, `conflict`, `invalid_skill`, `config_error`, `verification_failed`, and `operation_failed`.

```bash
skillfleet --json status
skillfleet --json endpoint ensure hermes ~/.hermes/skills
skillfleet --json skill ensure cleanup --source skills/cleanup --to hermes openclaw
skillfleet --json --sync --verify skill route-set cleanup --to hermes openclaw
skillfleet --json skill route-add cleanup --to pi
skillfleet --json skill route-remove cleanup --from openclaw
```

`status` returns config/library paths, endpoints, skills, routes, pending actions, summary counts, and health. Plan actions have stable names and a `destructive` flag. `add` rejects duplicates; use idempotent `ensure` in reconciliation loops. Legacy `skill route NAME --to ...` remains an exact-set alias. `--verify` implies `--sync`. Git subprocess output is suppressed in JSON mode.

## Commands

```text
skillfleet init --library PATH
skillfleet endpoint add NAME PATH
skillfleet endpoint ensure NAME PATH
skillfleet endpoint remove NAME
skillfleet endpoint list
skillfleet endpoint show NAME
skillfleet skill add NAME [--source PATH | --git URL [--subdir PATH]] [--to ...]
skillfleet skill ensure NAME [--source PATH | --git URL [--subdir PATH]] [--to ...]
skillfleet skill remove NAME
skillfleet skill route NAME --to [ENDPOINT ...]
skillfleet skill route-set NAME --to [ENDPOINT ...]
skillfleet skill route-add NAME --to ENDPOINT ...
skillfleet skill route-remove NAME --from ENDPOINT ...
skillfleet skill source NAME --for ENDPOINT PATH
skillfleet skill list
skillfleet skill show NAME
skillfleet plan
skillfleet status
skillfleet sync [--force]
skillfleet doctor
skillfleet tui
skillfleet update [NAME] [--check]
skillfleet self install --to [ENDPOINT ...]   # route the bundled skillfleet skill
skillfleet self update [--check]              # update the binaries from the latest release
# Global mutation options: --sync, --verify
```

Set `SKILLFLEET_CONFIG` or pass `--config`. Default: `~/.config/skillfleet/skillfleet.toml`.

## Safety model

- Skillfleet manages only links whose resolved targets are inside the configured library.
- Unrelated files and external symlinks in endpoint directories are ignored.
- `sync` refuses real-path conflicts.
- `sync --force` moves a conflicting path to a sibling `*.skillfleet-backup*` before linking.
- `plan` performs no writes.
- `doctor` requires every declared route to resolve to its exact canonical source.

## Status

`0.2.0`. MIT licensed. Linux/macOS symlink model. We are dogfooding it across Hermes, OpenClaw, Pi, OMP, Codex, Claude Code, OpenCode, Factory, and a generic Agent Skills endpoint before broadening the platform surface.
