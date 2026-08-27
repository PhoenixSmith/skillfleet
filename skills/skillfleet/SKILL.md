---
name: skillfleet
description: "Manage shared agent skills, endpoints, routing, and updates with Skillfleet."
version: 0.1.0
metadata:
  hermes:
    tags: [Skills, Agent-Skills, Skillfleet, Configuration]
---

# Skillfleet

Use Skillfleet whenever an agent needs to inspect, add, update, route, or audit shared Agent Skills. Skillfleet owns the routing manifest. Do not manually copy skill directories or create consumer-to-consumer symlinks.

## Operating model

- One configured library is canonical and should be git-backed.
- Owned skills live under the library, normally `skills/<name>/`.
- Third-party skills are vendored under `vendor/<name>/` by `skillfleet update`.
- Endpoints are arbitrary named directories such as `hermes`, `pi`, or `future-agent`.
- Skills target endpoint names. Skillfleet creates direct whole-directory symlinks from endpoints into the library.
- Per-endpoint source overrides support harness-specific skill variants.

## Agent-safe workflow

Always inspect before mutation:

```bash
skillfleet --json endpoint list
skillfleet --json skill list
skillfleet --json skill show <name>
skillfleet --json plan
```

Route a skill by replacing its exact target set:

```bash
skillfleet skill route <name> --to <endpoint> [endpoint...]
skillfleet --json plan
skillfleet sync
skillfleet --json doctor
```

Add an owned skill already present in the library:

```bash
skillfleet skill add <name> --source skills/<name> --to <endpoint...>
```

Subscribe to a Git-hosted skill:

```bash
skillfleet skill add <name> --git <url> [--subdir path/to/skill] --to <endpoint...>
skillfleet update <name>
skillfleet sync
skillfleet doctor
```

Set a per-endpoint source variant:

```bash
skillfleet skill source <name> --for <endpoint> skills/<name>/<variant>
```

## Safety rules

1. Use CLI commands, not direct manifest edits, unless repairing a malformed config.
2. Run `plan` before `sync` after routing changes.
3. Never use `sync --force` without inspecting each reported conflict. Force mode backs up real paths, but replacing an unreviewed directory can still hide active work.
4. `skillfleet update` changes vendored source. Inspect the library git diff before committing or trusting a third-party update.
5. Never route by copying. A valid managed endpoint entry is a direct symlink into the configured library.
6. Finish every mutation with `skillfleet --json doctor`. A nonzero exit means the setup is not clean.
7. Do not remove an endpoint while skills still target it. Re-route those skills first.
8. Keep the library git-clean after intentional changes are committed. Skillfleet does not push or publish repositories itself.

## Machine-readable behavior

Use global `--json` for discovery, plan, and doctor output. Commands are non-interactive and return nonzero on failure. Do not scrape human tables when JSON is available.

The config defaults to `~/.config/skillfleet/skillfleet.toml`. If the deployment uses a repository-local config, set `SKILLFLEET_CONFIG` or pass `--config <path>` on every invocation.
