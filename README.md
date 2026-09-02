<div align="center">
  <img src="assets/logo.svg" alt="Skillfleet logo" width="128" height="128">

  <h1>Skillfleet</h1>

  <p><strong>Git-backed skill distribution for AI agents.</strong><br>
  One canonical library. Arbitrary named endpoints. Explicit symlink routing.</p>

  <p>
    <a href="https://github.com/PhoenixSmith/skillfleet"><img src="https://img.shields.io/badge/version-0.3.0-f59e0b" alt="Version 0.3.0"></a>
    <img src="https://img.shields.io/badge/CLI-Rust-dea584?logo=rust&logoColor=white" alt="Rust CLI">
    <img src="https://img.shields.io/badge/TUI-Go%20%2B%20Bubble%20Tea-00ADD8?logo=go&logoColor=white" alt="Go TUI">
    <img src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS-4c9aff" alt="Linux and macOS">
    <img src="https://img.shields.io/badge/license-MIT-34d399" alt="MIT license">
  </p>
</div>

Every agent runtime reads skills from its own directory — `~/.claude/skills`, `~/.codex/skills`, `~/.hermes/skills`, and whatever ships next month. Without a manager that turns into copied directories, consumer-to-consumer symlinks, stale third-party snapshots, and edits landing in the wrong copy.

Skillfleet keeps every skill in one git-backed library and symlinks it into **named endpoints** — any directory an agent reads. No runtime is hardcoded: name the directory and target it.

```bash
skillfleet endpoint add claude ~/.claude/skills     # name a directory an agent reads
skillfleet skill add cleanup --source skills/cleanup --to claude codex hermes
skillfleet plan && skillfleet sync                  # inspect, then link
```

Edit a skill once; every endpoint sees it. Commit the library; your whole fleet is reproducible.

## Let your agents drive it

Skillfleet is built to be operated *by* agents, not just for them:

```bash
skillfleet self install --to claude codex hermes    # routes the bundled skillfleet skill
skillfleet sync
```

That teaches each agent the safe workflow — inspect with `status`, mutate through the CLI, never hand-copy skill directories. Then you can just tell your agent things like *"add this repo's review skill and route it everywhere"* or *"why is cleanup missing on codex?"*.

Every command is non-interactive, and `--json` emits exactly one document per run: `{ "schema_version": 1, "ok": true, "command": "...", "data": ... }` on success, `{ "ok": false, "error": { "code": "...", "message": "..." } }` on failure. Exit codes: `0` success, `1` operation failure, `2` invalid usage. Stable error codes (`already_exists`, `not_found`, `conflict`, …) and idempotent `ensure` variants make it safe in reconciliation loops and CI:

```bash
skillfleet --json status
skillfleet --json endpoint ensure hermes ~/.hermes/skills
skillfleet --json --sync --verify skill route-set cleanup --to hermes openclaw
```

## Features

- **One commit-able source of truth** — edit a skill once, every endpoint sees it
- **Arbitrary named endpoints** — no baked-in runtime compatibility table
- **Explicit per-skill routing** — each skill goes exactly where you send it
- **Git-sourced third-party skills** — vendored, pinned, updated with `skillfleet update`
- **Vacuum** — `sync` adopts skills you dropped into an endpoint by hand back into the library
- **Per-endpoint source variants** — different instructions per harness, same skill name
- **Dry inspection** — `plan` shows everything, writes nothing; `doctor` audits link health
- **Machine-readable `--json`** — built for agents and CI, not just humans
- **Safe conflict handling** — real directories are backed up, never silently deleted
- **Interactive TUI** — full-screen routing matrix with staged, reviewable changes

## Install

Prebuilt binaries for Linux and macOS (amd64/arm64), CLI + TUI in one command:

```bash
curl -fsSL https://raw.githubusercontent.com/PhoenixSmith/skillfleet/main/install.sh | sh
```

Installs `skillfleet` and `skillfleet-tui` into `~/.local/bin` (override with `SKILLFLEET_BINDIR`). The installer and `self update` verify the release against a published `SHA256SUMS` before extracting. Updates run only when you invoke them — no telemetry, no background phone-home.

From source (Rust + Go 1.18+): `make build`, `make test`, `make install PREFIX=/usr/local`. Plain `cargo install --path .` installs the CLI alone; build `skillfleet-tui` separately or set `SKILLFLEET_TUI`.

Shell completions and a man page are generated from the command model:

```bash
skillfleet completions bash        # or zsh, fish, elvish, powershell, nushell
skillfleet man                     # roff man page
skillfleet self uninstall --yes    # remove both binaries (prompts without --yes)
```

Pipe a completion script into your shell's completion directory, or write `make man` / `make completions` to stage them.

## Quick start

```bash
skillfleet init --library ~/agent-skills

skillfleet endpoint add claude ~/.claude/skills
skillfleet endpoint add codex ~/.codex/skills
skillfleet endpoint add hermes ~/.hermes/skills

skillfleet skill add cleanup --source skills/cleanup --to claude codex hermes

skillfleet plan      # dry run
skillfleet sync      # link
skillfleet doctor    # audit
```

New runtime next month? It's just another label:

```bash
skillfleet endpoint add future-agent ~/.future-agent/skills
skillfleet skill route-add cleanup --to future-agent
skillfleet sync
```

**Vacuum:** `sync` also adopts skills added to an endpoint by hand. A directory containing `SKILL.md` is copied into `<library>/skills/<name>`, registered for its originating endpoint only, and replaced by a managed symlink. Opt an endpoint out with `--no-vacuum` on `endpoint add`/`ensure` (`ensure` preserves the setting; `--vacuum` re-enables it). `plan` and `status` list pending adoptions as `vacuum_candidates`, so nothing is adopted that a dry run didn't predict. A directory whose name is already declared is never adopted — it surfaces as a plan conflict for `sync --force` to back up. Vacuum never commits or pushes the library repo.

## Third-party Git skills

Subscribe to a skill at a repo root, or a subdirectory of a larger repo:

```bash
skillfleet skill add autoreview \
  --git https://github.com/openclaw/agent-skills.git \
  --subdir skills/autoreview \
  --to hermes openclaw
skillfleet update autoreview && skillfleet sync
```

`update` vendors the directory under `<library>/vendor/<name>`; commit it to pin and review upstream changes. `skillfleet update && skillfleet sync && skillfleet doctor` is cron/CI-ready.

## Per-endpoint variants

Same skill name, different instructions per harness:

```bash
skillfleet skill add impeccable --source skills/impeccable/agents --to hermes claude
skillfleet skill source impeccable --for claude skills/impeccable/claude
```

## Interactive TUI

`skillfleet tui` opens a full-screen routing matrix over the same config and live symlink state. Toggle routes with `Space`; endpoint add/edit forms include a default-on **Vacuum manual skills** checkbox, also toggled with `Space`. Review grouped creates/removes/conflicts with `Ctrl+S` before anything touches disk. Apply shells out to the same CLI and finishes with a `doctor` audit. Each conflict is resolved explicitly (skip, keep, or backup-and-link). Press `?` for contextual help.

## Commands

```text
skillfleet init --library PATH
skillfleet endpoint add NAME PATH [--no-vacuum]
skillfleet endpoint ensure NAME PATH [--no-vacuum | --vacuum]
skillfleet endpoint remove NAME
skillfleet endpoint list | show NAME
skillfleet skill add NAME [--source PATH | --git URL [--subdir PATH]] [--to ...]
skillfleet skill ensure NAME [--source PATH | --git URL [--subdir PATH]] [--to ...]
skillfleet skill remove NAME                              # preserve canonical source
skillfleet skill delete NAME --global [--dry-run]         # preview or delete links, declaration, and source
skillfleet skill unroute NAME --from ENDPOINT ...          # canonical endpoint removal command
skillfleet skill route-set NAME --to [ENDPOINT ...]     # `skill route` is an alias
skillfleet skill route-add NAME --to ENDPOINT ...
skillfleet skill source NAME --for ENDPOINT PATH
skillfleet skill list | show NAME
skillfleet repair [--skill NAME] [--endpoint NAME] [--dry-run]
skillfleet plan | status | sync [--force] | doctor | tui
skillfleet update [NAME] [--check]
skillfleet self install --to [ENDPOINT ...]   # route the bundled skillfleet skill
skillfleet self update [--check]              # update the binaries from the latest release
skillfleet self uninstall [--yes]             # remove the binaries
skillfleet completions SHELL                  # bash/zsh/fish/elvish/powershell/nushell
skillfleet man                                # print the roff man page
# Global options: --json, --config PATH, --sync, --verify
```

Config resolution: `SKILLFLEET_CONFIG` or `--config` always wins. Otherwise the CLI and TUI walk up from the working directory looking for a repo-local `skillfleet.toml` (so a manifest committed beside the library is picked up on its own); with no manifest found it falls back to `~/.config/skillfleet/skillfleet.toml`. `skillfleet init` writes the manifest co-located at `{library}/skillfleet.toml` when no explicit path is given, so the routing config is versioned with the skills.

## Safety model

- Skillfleet manages only links whose resolved targets are inside the configured library.
- Vacuum adopts only directories containing `SKILL.md` on endpoints with vacuum enabled; unrelated files and external symlinks are ignored.
- `sync` refuses real-path conflicts; `sync --force` moves the conflicting path to a sibling `*.skillfleet-backup*` before linking.
- `plan` performs no writes.
- `doctor` requires every declared route to resolve to its exact canonical source.

## Status

`0.3.0`. MIT licensed. Linux/macOS symlink model. Dogfooded across Hermes, OpenClaw, Pi, OMP, Codex, Claude Code, OpenCode, Factory, and a generic Agent Skills endpoint before broadening the platform surface.
