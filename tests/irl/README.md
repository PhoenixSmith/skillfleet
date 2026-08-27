# IRL Live-Agent Verification Suite

End-to-end verification of Skillfleet against real agents in **fresh,
ephemeral containers** — not mocks. Each script installs a real agent (Hermes
or OpenClaw) from scratch, routes skills into it via Skillfleet, migrates a
pre-existing native skill, and confirms the agent actually sees and loads the
skills.

## Why

Unit tests prove the code is internally consistent. These prove the **end
user story**: "I installed skillfleet and a clean agent, and skills just show
up" — including the repo-local config default, the `init` export hint, and
vacuum migration of already-installed skills.

## What it covers

| Script | Agent | Verifies |
|--------|-------|----------|
| `01-cli-smoke.sh` | none | init co-locates manifest + prints export hint; full upsert→plan→sync→doctor; upward-walk config resolution; env-var location-independence; clean failure on unrelated cwd |
| `02-hermes-live.sh` | Hermes | fresh install; route 2 skills into `~/.hermes/skills`; vacuum-migrate a native real-dir skill; `hermes skills list` surfaces all 3 |
| `03-openclaw-live.sh` | OpenClaw | fresh install; route 2 skills into `~/.openclaw/skills`; vacuum-migrate a native skill; skills dir populated with symlinks |

The CLI smoke runs against `debian:12-slim` (fast). The agent scripts use the
full `debian:12` image because Hermes needs a compiler toolchain and OpenClaw
needs Node — installs take a few minutes each.

## Usage

```bash
tests/irl/run.sh            # full suite (all 3)
tests/irl/run.sh --cli      # fast CLI-only smoke
tests/irl/run.sh --hermes   # Hermes live only
tests/irl/run.sh --openclaw # OpenClaw live only
```

Requires `docker` and network access (fetches agent installers + base images).
Builds the release binaries first via `make build`. Each test runs in its own
fresh container and is torn down after.

## Adding a new agent

Copy `02-hermes-live.sh` → `0N-<agent>-live.sh`. Key adaptations:
1. Install the agent non-interactively (set `--no-onboard`/`--skip-setup` flags).
2. Point the skillfleet endpoint at the agent's skills directory (OpenClaw
   reads `~/.openclaw/skills`; Hermes reads `~/.hermes/skills`).
3. Keep the shared flow: init → co-located manifest → route 2 → native
   migration → doctor clean → prove the agent lists the skills.
4. Register it in `run.sh`'s `run_in` block.

## Note on "agent consumes the skills"

The strongest proof of live consumption is an actual agent turn that loads a
skill (`hermes chat`/`openclaw run`), which needs an LLM API key. The suite
stops at the next-best deterministic proof: the skills symlink into the exact
managed directory each agent reads, and the loader surfaces them in the
agent's own skill listing. For symlink behavior, both agents accept
manager-owned symlinked skill roots unconditionally per their docs.