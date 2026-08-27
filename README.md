# Skillfleet

Git-backed skill distribution for AI agents.

Skillfleet keeps skills in one canonical library and routes whole skill directories into arbitrary **named endpoints** using symlinks. It does not hardcode Hermes, OpenClaw, Claude Code, Codex, Pi, or any other runtime. If an agent reads skills from a directory, name that directory and target it.

## Why

Without a manager, multi-agent skill setups become a thicket of copied directories, consumer-to-consumer symlinks, stale third-party snapshots, and edits landing in the wrong place.

Skillfleet provides:

- one commit-able source of truth;
- arbitrary named endpoints;
- explicit per-skill routing;
- owned/personal skills;
- Git-sourced third-party skills with updates;
- per-endpoint source variants;
- dry inspection with `plan`;
- machine-readable `--json` output;
- safe conflict handling: real directories are backed up, never silently deleted.

## Install

```bash
cargo install --path .
```

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

Commands are non-interactive and return nonzero on failure. Add `--json` globally:

```bash
skillfleet --json endpoint list
skillfleet --json skill list
skillfleet --json skill show cleanup
skillfleet --json plan
skillfleet --json doctor
```

Recommended agent loop:

1. `skillfleet --json endpoint list`
2. `skillfleet --json skill show <name>`
3. `skillfleet skill route <name> --to <endpoints...>`
4. `skillfleet --json plan`
5. `skillfleet sync`
6. `skillfleet --json doctor`

## Commands

```text
skillfleet init --library PATH
skillfleet endpoint add NAME PATH
skillfleet endpoint remove NAME
skillfleet endpoint list
skillfleet endpoint show NAME
skillfleet skill add NAME [--source PATH | --git URL [--subdir PATH]] [--to ...]
skillfleet skill remove NAME
skillfleet skill route NAME --to [ENDPOINT ...]
skillfleet skill source NAME --for ENDPOINT PATH
skillfleet skill list
skillfleet skill show NAME
skillfleet plan
skillfleet sync [--force]
skillfleet doctor
skillfleet update [NAME]
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

`0.1.0` beta. Linux/macOS symlink model. We are dogfooding it across Hermes, OpenClaw, Pi, OMP, Codex, Claude Code, OpenCode, Factory, and a generic Agent Skills endpoint before broadening the platform surface.
