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
- `sync` vacuums manually added skill directories into `skills/<name>/` by default, registers them only for the originating endpoint, and links them back. Use `endpoint add/ensure ... --no-vacuum` for a local-only endpoint.
- `plan` and `status` list pending adoptions under `vacuum_candidates` (with a `conflict` flag for already-declared names), so inspect before `sync` to know exactly what it will adopt.
- The TUI endpoint add/edit form exposes the same default-on vacuum setting as a checkbox; Space toggles it.

## Agent-safe workflow

Always inspect before mutation. `status` is the preferred single-call snapshot:

```bash
skillfleet --json status
skillfleet --json skill show <name>
skillfleet --json plan
```

JSON mode emits one versioned success or error envelope. Treat exit 1 as an operation/verification failure and exit 2 as invalid usage; parse `error.code`, not human text.

Route explicitly by setting, adding, or removing targets. For an atomic agent mutation, use global `--sync --verify` (`--verify` implies sync):

```bash
skillfleet --json --sync --verify skill route-set <name> --to <endpoint> [endpoint...]
skillfleet --json skill route-add <name> --to <endpoint...>
skillfleet --json skill route-remove <name> --from <endpoint...>
```

Legacy `skill route` is exact-set routing. Prefer `route-set` in new automation.

Reconcile endpoints and owned skills idempotently. `add` intentionally fails if the name exists:

```bash
skillfleet --json endpoint ensure <endpoint> <path>
skillfleet --json skill ensure <name> --source skills/<name> --to <endpoint...>
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
9. Vacuum is default-on and never adopts a name that is already declared — such directories surface as plan conflicts for `sync --force` to back up. It changes the local library and config only; inspect and commit those changes separately.

## Machine-readable behavior

Use global `--json` for discovery, plan, and doctor output. Commands are non-interactive and return nonzero on failure. Do not scrape human tables when JSON is available.

Config resolution: `SKILLFLEET_CONFIG` or `--config` always wins; otherwise the CLI and TUI walk up from the working directory for a repo-local `skillfleet.toml` (a manifest committed beside the library is picked up automatically), falling back to `~/.config/skillfleet/skillfleet.toml`. `skillfleet init` writes the manifest at `{library}/skillfleet.toml` when no explicit path is given, so the routing config is versioned with the skills. Point-invariant helpers (cron, CI, agents) should still pass the config or set `SKILLFLEET_CONFIG` explicitly rather than depend on the working directory.

Use `skillfleet --json update <name> --check` to preview old/new revisions and changed files without replacing vendored content. Plan output includes stable action names, destructive markers, source validation errors, and summary counts.
