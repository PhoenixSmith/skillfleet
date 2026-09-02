use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Parser)]
#[command(
    name = "skillfleet",
    version,
    about = "Git-backed skill routing for AI agents"
)]
struct Cli {
    /// Config file path (default: skillfleet.toml in the working directory, else ~/.config/skillfleet/skillfleet.toml).
    #[arg(long, global = true, env = "SKILLFLEET_CONFIG")]
    config: Option<PathBuf>,
    /// Emit exactly one machine-readable JSON document on stdout.
    #[arg(long, global = true)]
    json: bool,
    /// Run a safe sync after a successful mutation.
    #[arg(long = "sync", global = true)]
    sync_after: bool,
    /// Sync after a successful mutation, then verify with doctor (implies --sync).
    #[arg(long, global = true)]
    verify: bool,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a new config pointing at a canonical skill library directory.
    Init {
        /// Directory that holds (or will hold) the canonical skill library.
        #[arg(long)]
        library: PathBuf,
    },
    /// Manage named endpoints: the directories agents read skills from.
    Endpoint {
        #[command(subcommand)]
        command: EndpointCmd,
    },
    /// Manage skills: sources, routing, and per-endpoint overrides.
    Skill {
        #[command(subcommand)]
        command: SkillCmd,
    },
    /// Show everything sync would do, without writing anything.
    Plan,
    /// One-call snapshot: config, endpoints, skills, routes, plan, and health.
    Status,
    /// Create or repair links; adopts manual skills from vacuum-enabled endpoints.
    Sync {
        /// Move conflicting real paths to a *.skillfleet-backup* sibling before linking.
        #[arg(long)]
        force: bool,
    },
    /// Verify every declared route resolves to its exact canonical source.
    Doctor,
    /// Open the interactive Bubble Tea routing inspector.
    Tui,
    /// Generate a shell completion script and print it to stdout.
    Completions {
        /// bash, zsh, fish, elvish, powershell, or nushell.
        #[arg(value_parser = clap::value_parser!(clap_complete::Shell))]
        shell: clap_complete::Shell,
    },
    /// Print the roff man page to stdout.
    Man,
    /// Refresh vendored copies of git-sourced skills.
    Update {
        /// Skill to update (default: every git-sourced skill).
        name: Option<String>,
        /// Report available updates without changing anything.
        #[arg(long)]
        check: bool,
    },
    /// Manage the skillfleet installation and its bundled agent skill.
    #[command(name = "self")]
    SelfSkill {
        #[command(subcommand)]
        command: SelfCmd,
    },
}

#[derive(Subcommand)]
enum SelfCmd {
    /// Route the bundled skillfleet skill to the given endpoints, teaching agents to operate skillfleet.
    Install {
        /// Endpoint names that receive the bundled skill.
        #[arg(long, num_args = 1..)]
        to: Vec<String>,
    },
    /// Check for and install the latest released binary.
    Update {
        /// Report whether an update is available without installing.
        #[arg(long)]
        check: bool,
    },
    /// Remove the skillfleet and skillfleet-tui binaries.
    Uninstall {
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum EndpointCmd {
    /// Register a new named endpoint at a directory (fails if the name exists).
    Add {
        /// Endpoint name: letters, digits, '-', '_', '.'.
        name: String,
        /// Directory the agent reads skills from.
        path: PathBuf,
        /// Do not adopt manually added skills from this endpoint during sync.
        #[arg(long)]
        no_vacuum: bool,
    },
    /// Idempotently create or update an endpoint; keeps its vacuum setting unless a flag overrides it.
    Ensure {
        /// Endpoint name: letters, digits, '-', '_', '.'.
        name: String,
        /// Directory the agent reads skills from.
        path: PathBuf,
        /// Do not adopt manually added skills from this endpoint during sync.
        #[arg(long)]
        no_vacuum: bool,
        /// Re-enable adopting manually added skills from this endpoint during sync.
        #[arg(long, conflicts_with = "no_vacuum")]
        vacuum: bool,
    },
    /// Remove an endpoint from the config.
    Remove { name: String },
    /// List endpoints with their paths.
    List,
    /// Show one endpoint's configuration.
    Show { name: String },
}

#[derive(Subcommand)]
enum SkillCmd {
    /// Declare a skill from a library path or a git remote (fails if the name exists).
    Add(SkillAdd),
    /// Idempotently declare or update a skill; safe in reconciliation loops.
    Ensure(SkillAdd),
    /// Remove a skill declaration and all managed endpoint links, preserving source content.
    Remove { name: String },
    /// Remove a skill from selected endpoints while preserving its declaration and source.
    Unroute {
        name: String,
        #[arg(long, num_args=1..)]
        from: Vec<String>,
    },
    /// Delete a skill globally: managed endpoint links, declaration, and canonical source.
    Delete {
        name: String,
        /// Required acknowledgement that canonical source content will be deleted.
        #[arg(long)]
        global: bool,
    },
    /// Alias of route-set: replace the full set of endpoints a skill targets.
    Route {
        name: String,
        /// Endpoint names; pass none to clear all routes.
        #[arg(long, num_args=0..)]
        to: Vec<String>,
    },
    /// Replace the full set of endpoints a skill targets.
    RouteSet {
        name: String,
        /// Endpoint names; pass none to clear all routes.
        #[arg(long, num_args=0..)]
        to: Vec<String>,
    },
    /// Add endpoints to a skill's targets.
    RouteAdd {
        name: String,
        #[arg(long, num_args=1..)]
        to: Vec<String>,
    },
    /// Remove endpoints from a skill's targets.
    RouteRemove {
        name: String,
        #[arg(long, num_args=1..)]
        from: Vec<String>,
    },
    /// Set a per-endpoint source override for a harness-specific skill variant.
    Source {
        name: String,
        /// Endpoint that gets the override.
        #[arg(long = "for")]
        endpoint: String,
        /// Source path for that endpoint (library-relative or absolute).
        path: PathBuf,
    },
    /// List skills and their targets.
    List,
    /// Show one skill's declaration.
    Show { name: String },
}

#[derive(Args)]
struct SkillAdd {
    /// Skill name: letters, digits, '-', '_', '.'.
    name: String,
    /// Library-relative (or absolute) path to the skill directory.
    #[arg(long, conflicts_with = "git")]
    source: Option<PathBuf>,
    /// Git URL to vendor the skill from.
    #[arg(long, conflicts_with = "source")]
    git: Option<String>,
    /// Directory inside the git repository that holds the skill.
    #[arg(long)]
    subdir: Option<PathBuf>,
    /// Endpoint names to route the skill to.
    #[arg(long, num_args=0..)]
    to: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Config {
    schema: u32,
    library: PathBuf,
    #[serde(default)]
    endpoints: BTreeMap<String, Endpoint>,
    #[serde(default)]
    skills: BTreeMap<String, Skill>,
}
#[derive(Debug, Serialize, Deserialize)]
struct Endpoint {
    path: PathBuf,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    vacuum: bool,
}

fn default_true() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}
#[derive(Debug, Serialize, Deserialize)]
struct Skill {
    source: PathBuf,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    source_overrides: BTreeMap<String, PathBuf>,
    #[serde(default)]
    targets: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remote: Option<Remote>,
}
#[derive(Debug, Serialize, Deserialize)]
struct Remote {
    git: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    subdir: Option<PathBuf>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ActionKind {
    Ok,
    Link,
    Relink,
    Unlink,
    Conflict,
    Error,
}

#[derive(Debug, Serialize)]
struct Action {
    action: ActionKind,
    destructive: bool,
    skill: String,
    endpoint: String,
    destination: PathBuf,
    source: Option<PathBuf>,
    detail: Option<String>,
}

const SELF_SKILL: &str = include_str!("../skills/skillfleet/SKILL.md");

fn default_config() -> PathBuf {
    // Prefer a repo-local manifest committed with the library: walk up from the
    // working directory so a config checked in alongside the skills is picked
    // up automatically (the git-backed source-of-truth pattern). Fall back to
    // the per-user XDG location.
    if let Some(p) = find_upward_manifest(std::env::current_dir().ok()) {
        return p;
    }
    dirs_home().join(".config/skillfleet/skillfleet.toml")
}
fn find_upward_manifest(start: Option<PathBuf>) -> Option<PathBuf> {
    let mut dir = start?;
    loop {
        let candidate = dir.join("skillfleet.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}
fn co_located_config(library: &Path) -> PathBuf {
    expand(library).join("skillfleet.toml")
}
fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
fn expand(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if s == "~" {
        return dirs_home();
    }
    if let Some(rest) = s.strip_prefix("~/") {
        return dirs_home().join(rest);
    }
    p.to_path_buf()
}
fn load(path: &Path) -> Result<Config> {
    if !path.exists() {
        bail!(
            "no config file at {}; run 'skillfleet init --library PATH' to create one",
            path.display()
        );
    }
    let text =
        fs::read_to_string(path).with_context(|| format!("read config {}", path.display()))?;
    let cfg: Config = toml::from_str(&text).context("parse config")?;
    if cfg.schema != 1 {
        bail!("unsupported schema {}, expected 1", cfg.schema);
    }
    Ok(cfg)
}
fn save(path: &Path, cfg: &Config) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, toml::to_string_pretty(cfg)?)?;
    fs::rename(tmp, path)?;
    Ok(())
}
fn source_path(cfg: &Config, skill: &Skill, endpoint: Option<&str>) -> PathBuf {
    let configured = endpoint
        .and_then(|name| skill.source_overrides.get(name))
        .unwrap_or(&skill.source);
    let p = expand(configured);
    if p.is_absolute() {
        p
    } else {
        expand(&cfg.library).join(p)
    }
}
fn normalize_targets(targets: &mut Vec<String>) {
    targets.sort();
    targets.dedup();
}
fn error_code(message: &str) -> &'static str {
    if message.contains("doctor found") {
        "verification_failed"
    } else if message.contains("already exists") {
        "already_exists"
    } else if message.contains("not found") {
        "not_found"
    } else if message.contains("unknown endpoint") {
        "unknown_endpoint"
    } else if message.contains("conflict") {
        "conflict"
    } else if message.contains("missing SKILL.md") || message.contains("has no SKILL.md") {
        "invalid_skill"
    } else if message.contains("config") {
        "config_error"
    } else {
        "operation_failed"
    }
}
fn validate_name(s: &str) -> Result<()> {
    if s.is_empty()
        || !s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        bail!("invalid name: {s}");
    }
    Ok(())
}
fn ensure_targets(cfg: &Config, targets: &[String]) -> Result<()> {
    let unknown: Vec<_> = targets
        .iter()
        .filter(|t| !cfg.endpoints.contains_key(*t))
        .collect();
    if !unknown.is_empty() {
        bail!(
            "unknown endpoints: {}",
            unknown
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(())
}
fn is_managed_link(path: &Path, cfg: &Config) -> bool {
    if !path.is_symlink() {
        return false;
    }

    // canonicalize() alone cannot classify a dangling symlink. Managed links
    // become dangling precisely when a skill source is removed out of order,
    // and must still be recognized so sync can clean them up.
    if fs::canonicalize(path)
        .ok()
        .is_some_and(|p| p.starts_with(expand(&cfg.library)))
    {
        return true;
    }

    let Ok(target) = fs::read_link(path) else {
        return false;
    };
    let target = if target.is_absolute() {
        target
    } else {
        path.parent().unwrap_or_else(|| Path::new(".")).join(target)
    };
    let library = expand(&cfg.library);
    target.starts_with(&library)
}
fn plan(cfg: &Config) -> Result<Vec<Action>> {
    let mut out = Vec::new();
    for (name, skill) in &cfg.skills {
        for target in &skill.targets {
            let src = source_path(cfg, skill, Some(target));
            if !src.join("SKILL.md").is_file() {
                out.push(Action {
                    action: ActionKind::Error,
                    destructive: false,
                    skill: name.clone(),
                    endpoint: target.clone(),
                    destination: cfg
                        .endpoints
                        .get(target)
                        .map(|e| expand(&e.path).join(name))
                        .unwrap_or_default(),
                    source: Some(src),
                    detail: Some("source missing SKILL.md".into()),
                });
                continue;
            }
            let Some(ep) = cfg.endpoints.get(target) else {
                out.push(Action {
                    action: ActionKind::Error,
                    destructive: false,
                    skill: name.clone(),
                    endpoint: target.clone(),
                    destination: PathBuf::new(),
                    source: Some(src.clone()),
                    detail: Some("unknown endpoint".into()),
                });
                continue;
            };
            let dst = expand(&ep.path).join(name);
            match fs::symlink_metadata(&dst) {
                Err(_) => out.push(Action {
                    action: ActionKind::Link,
                    destructive: false,
                    skill: name.clone(),
                    endpoint: target.clone(),
                    destination: dst,
                    source: Some(src.clone()),
                    detail: Some(
                        "declared route missing; run skillfleet sync to restore it, or skillfleet skill unroute <name> --from <endpoint> --sync to uninstall it from this endpoint"
                            .into(),
                    ),
                }),
                Ok(meta) if meta.file_type().is_symlink() => {
                    let actual = fs::canonicalize(&dst).ok();
                    let expected = fs::canonicalize(&src).ok();
                    if actual == expected {
                        out.push(Action {
                            action: ActionKind::Ok,
                            destructive: false,
                            skill: name.clone(),
                            endpoint: target.clone(),
                            destination: dst,
                            source: Some(src.clone()),
                            detail: None,
                        });
                    } else if is_managed_link(&dst, cfg) {
                        out.push(Action {
                            action: ActionKind::Relink,
                            destructive: true,
                            skill: name.clone(),
                            endpoint: target.clone(),
                            destination: dst,
                            source: Some(src.clone()),
                            detail: None,
                        });
                    } else {
                        out.push(Action {
                            action: ActionKind::Conflict,
                            destructive: true,
                            skill: name.clone(),
                            endpoint: target.clone(),
                            destination: dst,
                            source: Some(src.clone()),
                            detail: Some("external or broken symlink exists".into()),
                        });
                    }
                }
                Ok(_) => out.push(Action {
                    action: ActionKind::Conflict,
                    destructive: true,
                    skill: name.clone(),
                    endpoint: target.clone(),
                    destination: dst,
                    source: Some(src.clone()),
                    detail: Some("real path exists".into()),
                }),
            }
        }
    }
    for (ep_name, ep) in &cfg.endpoints {
        let root = expand(&ep.path);
        let declared: BTreeSet<_> = cfg
            .skills
            .iter()
            .filter(|(_, s)| s.targets.contains(ep_name))
            .map(|(n, _)| n.as_str())
            .collect();
        if let Ok(entries) = fs::read_dir(&root) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let p = entry.path();
                if !declared.contains(name.as_str()) && is_managed_link(&p, cfg) {
                    out.push(Action {
                        action: ActionKind::Unlink,
                        destructive: true,
                        skill: name,
                        endpoint: ep_name.clone(),
                        destination: p,
                        source: None,
                        detail: None,
                    });
                }
            }
        }
    }
    Ok(out)
}
fn backup_path(dst: &Path) -> PathBuf {
    let mut p = dst.with_extension("skillfleet-backup");
    let mut n = 1;
    while p.exists() {
        p = dst.with_extension(format!("skillfleet-backup-{n}"));
        n += 1;
    }
    p
}
fn delete_skill(
    cfg: &mut Config,
    config_path: &Path,
    name: &str,
    global: bool,
) -> Result<serde_json::Value> {
    if !global {
        bail!("global deletion requires --global; use 'skill remove' to preserve source content");
    }
    let skill = cfg
        .skills
        .get(name)
        .with_context(|| format!("skill not found: {name}"))?;
    let library = expand(&cfg.library);
    let canonical_library = fs::canonicalize(&library).unwrap_or_else(|_| library.clone());

    let mut sources = vec![source_path(cfg, skill, None)];
    sources.extend(skill.source_overrides.values().map(|source| {
        if source.is_absolute() {
            source.clone()
        } else {
            library.join(source)
        }
    }));
    sources.sort();
    sources.dedup();
    for source in &sources {
        let comparable = fs::canonicalize(source).unwrap_or_else(|_| source.clone());
        if comparable == canonical_library || !comparable.starts_with(&canonical_library) {
            bail!(
                "refusing to delete source outside the canonical library: {}",
                source.display()
            );
        }
    }
    for (other_name, other) in &cfg.skills {
        if other_name == name {
            continue;
        }
        let mut other_sources = vec![source_path(cfg, other, None)];
        other_sources.extend(other.source_overrides.values().map(|source| {
            if source.is_absolute() {
                source.clone()
            } else {
                library.join(source)
            }
        }));
        if let Some(shared) = sources.iter().find(|source| other_sources.contains(source)) {
            bail!(
                "refusing to delete source shared with skill {other_name}: {}",
                shared.display()
            );
        }
    }

    let mut links = Vec::new();
    for target in &skill.targets {
        let endpoint = cfg
            .endpoints
            .get(target)
            .with_context(|| format!("unknown endpoint: {target}"))?;
        let destination = expand(&endpoint.path).join(name);
        match fs::symlink_metadata(&destination) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("inspect endpoint link before delete"),
            Ok(metadata)
                if metadata.file_type().is_symlink() && is_managed_link(&destination, cfg) =>
            {
                links.push(destination)
            }
            Ok(_) => bail!(
                "refusing global delete: unmanaged or conflicting endpoint path exists: {}",
                destination.display()
            ),
        }
    }

    // Persist intent before destructive filesystem cleanup. If saving fails,
    // no links or source content have been touched. Any later cleanup failure
    // is recoverable because sync recognizes undeclared managed links.
    cfg.skills.remove(name);
    save(config_path, cfg)?;

    let mut removed_links = Vec::new();
    for link in links {
        fs::remove_file(&link)
            .with_context(|| format!("remove managed endpoint link {}", link.display()))?;
        removed_links.push(link);
    }
    let mut removed_sources = Vec::new();
    // Delete children before parents when an override lives inside the base source.
    sources.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for source in sources {
        if source.is_dir() {
            fs::remove_dir_all(&source)
                .with_context(|| format!("delete canonical source {}", source.display()))?;
            removed_sources.push(source);
        } else if source.is_file() || source.is_symlink() {
            fs::remove_file(&source)
                .with_context(|| format!("delete canonical source {}", source.display()))?;
            removed_sources.push(source);
        }
    }
    Ok(serde_json::json!({
        "name": name,
        "global": true,
        "removed_links": removed_links,
        "removed_sources": removed_sources,
    }))
}

fn apply(cfg: &Config, force: bool) -> Result<Vec<Action>> {
    let actions = plan(cfg)?;

    // Validate the complete deterministic plan before mutating any endpoint.
    // Without this preflight, an earlier Link/Unlink could succeed before a
    // later missing source or conflict made sync return an error, leaving a
    // partially-applied deployment.
    if let Some(a) = actions
        .iter()
        .find(|a| matches!(a.action, ActionKind::Error))
    {
        bail!("{}", a.detail.as_deref().unwrap_or("plan error"));
    }
    if let Some(a) = actions
        .iter()
        .find(|a| !force && matches!(a.action, ActionKind::Conflict))
    {
        bail!(
            "conflict at {}; rerun sync --force to back it up",
            a.destination.display()
        );
    }

    for a in &actions {
        match a.action {
            ActionKind::Link => {
                let src = a.source.as_ref().unwrap();
                if !src.join("SKILL.md").is_file() {
                    bail!("{} missing SKILL.md", src.display())
                }
                fs::create_dir_all(a.destination.parent().unwrap())?;
                symlink(src, &a.destination)?;
            }
            ActionKind::Relink => {
                let src = a.source.as_ref().unwrap();
                if !src.join("SKILL.md").is_file() {
                    bail!("{} missing SKILL.md", src.display());
                }
                fs::remove_file(&a.destination)?;
                symlink(src, &a.destination)?;
            }
            ActionKind::Unlink => fs::remove_file(&a.destination)?,
            ActionKind::Conflict if force => {
                let src = a.source.as_ref().unwrap();
                if !src.join("SKILL.md").is_file() {
                    bail!("{} missing SKILL.md", src.display());
                }
                let b = backup_path(&a.destination);
                fs::rename(&a.destination, &b)?;
                symlink(src, &a.destination)?;
            }
            ActionKind::Conflict => bail!(
                "conflict at {}; rerun sync --force to back it up",
                a.destination.display()
            ),
            ActionKind::Error => bail!("{}", a.detail.as_deref().unwrap_or("plan error")),
            _ => {}
        }
    }
    Ok(actions)
}
#[derive(Debug, Serialize)]
struct UpdateReport {
    name: String,
    check: bool,
    old_revision: Option<String>,
    new_revision: String,
    changed_files: Vec<String>,
    changed: bool,
}

fn git_output(args: &[&str], cwd: Option<&Path>) -> Result<String> {
    let mut command = Command::new("git");
    command
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    if let Some(path) = cwd {
        command.current_dir(path);
    }
    let output = command.output()?;
    if !output.status.success() {
        bail!("git command failed: git {}", args.join(" "));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn copy_skill_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git" || name == ".skillfleet-revision" {
            continue;
        }
        let source_path = entry.path();
        let destination_path = destination.join(name);
        if entry.file_type()?.is_dir() {
            copy_skill_tree(&source_path, &destination_path)?;
        } else {
            fs::copy(source_path, destination_path)?;
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct VacuumReport {
    name: String,
    endpoint: String,
    source: PathBuf,
}

#[derive(Debug, Serialize)]
struct VacuumCandidate {
    name: String,
    endpoint: String,
    source: PathBuf,
    conflict: bool,
}

/// Read-only scan for manual skill directories that `sync` would adopt:
/// plain directories (never symlinks) holding a SKILL.md, with a manageable
/// name, inside vacuum-enabled endpoints. `conflict` marks names that are
/// already declared, which `sync` refuses to adopt.
fn vacuum_candidates(cfg: &Config) -> Vec<VacuumCandidate> {
    let mut out = Vec::new();
    for (endpoint_name, endpoint) in &cfg.endpoints {
        if !endpoint.vacuum {
            continue;
        }
        let root = expand(&endpoint.path);
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if !file_type.is_dir() || file_type.is_symlink() || !path.join("SKILL.md").is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            // Never adopt `sync --force` backups: re-adopting what was just
            // moved aside would resurrect the conflict as a bogus skill.
            if validate_name(&name).is_err() || name.contains(".skillfleet-backup") {
                continue;
            }
            out.push(VacuumCandidate {
                conflict: cfg.skills.contains_key(&name),
                endpoint: endpoint_name.clone(),
                name,
                source: path,
            });
        }
    }
    out.sort_by(|a, b| (&a.endpoint, &a.name).cmp(&(&b.endpoint, &b.name)));
    out
}

fn vacuum_endpoints(cfg: &mut Config, config_path: &Path) -> Result<Vec<VacuumReport>> {
    let mut reports = Vec::new();
    for candidate in vacuum_candidates(cfg) {
        let (endpoint, name, origin) = (candidate.endpoint, candidate.name, candidate.source);
        // A directory shadowing a declared skill is never adopted; the plan
        // reports it as a conflict and `sync --force` backs it up before
        // linking. Bailing here would block conflict resolution entirely.
        if cfg.skills.contains_key(&name) {
            continue;
        }
        let relative = PathBuf::from(format!("skills/{name}"));
        let destination = expand(&cfg.library).join(&relative);
        if fs::symlink_metadata(&destination).is_ok() {
            bail!(
                "vacuum conflict: library destination already exists: {}",
                destination.display()
            );
        }
        let parent = destination
            .parent()
            .context("vacuum destination has no parent")?;
        fs::create_dir_all(parent)?;
        let temp = parent.join(format!(".{name}.skillfleet-vacuum-tmp"));
        if fs::symlink_metadata(&temp).is_ok() {
            bail!("vacuum temporary path already exists: {}", temp.display());
        }
        copy_skill_tree(&origin, &temp)
            .with_context(|| format!("vacuum {name} from endpoint {endpoint}"))?;
        if !temp.join("SKILL.md").is_file() {
            let _ = fs::remove_dir_all(&temp);
            bail!("vacuum copy for {name} is missing SKILL.md");
        }
        fs::rename(&temp, &destination)?;
        cfg.skills.insert(
            name.clone(),
            Skill {
                source: relative,
                source_overrides: BTreeMap::new(),
                targets: vec![endpoint.clone()],
                remote: None,
            },
        );
        if let Err(error) = save(config_path, cfg) {
            cfg.skills.remove(&name);
            let _ = fs::remove_dir_all(&destination);
            return Err(error).context("save config after vacuum");
        }
        fs::remove_dir_all(&origin)?;
        if let Err(error) = symlink(&destination, &origin) {
            return Err(error)
                .with_context(|| format!("link vacuumed skill {name} back to {endpoint}"));
        }
        reports.push(VacuumReport {
            name,
            endpoint,
            source: destination,
        });
    }
    Ok(reports)
}

fn update_remote(cfg: &Config, name: &str, skill: &Skill, check: bool) -> Result<UpdateReport> {
    let remote = skill.remote.as_ref().context("skill is not remote")?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let checkout =
        std::env::temp_dir().join(format!("skillfleet-{name}-{}-{nonce}", std::process::id()));
    let output = Command::new("git")
        .args(["clone", "--depth", "1", &remote.git])
        .arg(&checkout)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .output()?;
    if !output.status.success() {
        bail!("git clone failed for {name}");
    }
    let result = (|| {
        let new_revision = git_output(&["rev-parse", "HEAD"], Some(&checkout))?;
        let selected = remote
            .subdir
            .as_ref()
            .map(|p| checkout.join(p))
            .unwrap_or(checkout.clone());
        if !selected.join("SKILL.md").is_file() {
            bail!("remote {name} has no SKILL.md at {}", selected.display());
        }
        let dst = source_path(cfg, skill, None);
        let old_revision = fs::read_to_string(dst.join(".skillfleet-revision"))
            .ok()
            .map(|s| s.trim().to_owned());
        let changed_files = if dst.exists() {
            let output = Command::new("git")
                .args(["diff", "--no-index", "--name-only", "--"])
                .arg(&dst)
                .arg(&selected)
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()?;
            let dst_text = dst.to_string_lossy();
            let selected_text = selected.to_string_lossy();
            let mut files: Vec<String> = String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| {
                    let unquoted = line.trim_matches('"');
                    let relative = unquoted
                        .strip_prefix(dst_text.as_ref())
                        .or_else(|| unquoted.strip_prefix(selected_text.as_ref()))
                        .unwrap_or(unquoted)
                        .trim_start_matches('/');
                    (relative != ".skillfleet-revision"
                        && relative != "dev/null"
                        && !relative.starts_with(".git/"))
                    .then(|| relative.to_owned())
                })
                .collect();
            files.sort();
            files.dedup();
            files
        } else {
            vec!["SKILL.md".into()]
        };
        let changed = !changed_files.is_empty();
        if !check && changed {
            let tmp = dst.with_extension("update-tmp");
            if tmp.exists() {
                fs::remove_dir_all(&tmp)?;
            }
            fs::create_dir_all(tmp.parent().context("vendor destination has no parent")?)?;
            copy_skill_tree(&selected, &tmp)?;
            fs::write(
                tmp.join(".skillfleet-revision"),
                format!("{new_revision}\n"),
            )?;
            if dst.exists() {
                fs::remove_dir_all(&dst)?;
            }
            fs::rename(tmp, &dst)?;
        }
        Ok(UpdateReport {
            name: name.into(),
            check,
            old_revision,
            new_revision,
            changed_files,
            changed,
        })
    })();
    if checkout.exists() {
        let _ = fs::remove_dir_all(&checkout);
    }
    result
}

fn action_label(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::Ok => "ok",
        ActionKind::Link => "link",
        ActionKind::Relink => "relink",
        ActionKind::Unlink => "unlink",
        ActionKind::Conflict => "conflict",
        ActionKind::Error => "error",
    }
}

fn plan_document(actions: &[Action]) -> serde_json::Value {
    let mut counts = BTreeMap::<&str, usize>::new();
    for action in actions {
        *counts.entry(action_label(action.action)).or_default() += 1;
    }
    serde_json::json!({"schema_version": 1, "actions": actions, "summary": {
        "total": actions.len(), "counts": counts,
        "healthy": actions.iter().all(|a| a.action == ActionKind::Ok),
        "destructive": actions.iter().filter(|a| a.destructive).count()
    }})
}

#[derive(Debug)]
struct Outcome {
    command: String,
    data: serde_json::Value,
    human: String,
    mutated: bool,
    /// Extra human-only guidance printed after `human` (e.g. a shell export to
    /// pin the config path). Not part of the JSON envelope.
    hint: Option<String>,
}

fn skill_from_add(a: SkillAdd) -> Skill {
    let mut targets = a.to;
    normalize_targets(&mut targets);
    if let Some(url) = a.git {
        Skill {
            source: PathBuf::from(format!("vendor/{}", a.name)),
            source_overrides: BTreeMap::new(),
            targets,
            remote: Some(Remote {
                git: url,
                subdir: a.subdir,
            }),
        }
    } else {
        Skill {
            source: a
                .source
                .unwrap_or_else(|| PathBuf::from(format!("skills/{}", a.name))),
            source_overrides: BTreeMap::new(),
            targets,
            remote: None,
        }
    }
}

fn upsert_skill(cfg: &mut Config, path: &Path, a: SkillAdd, ensuring: bool) -> Result<Outcome> {
    validate_name(&a.name)?;
    ensure_targets(cfg, &a.to)?;
    let name = a.name.clone();
    let skill = skill_from_add(a);
    if !ensuring && cfg.skills.contains_key(&name) {
        bail!("skill already exists: {name}");
    }
    let old = cfg
        .skills
        .get(&name)
        .map(serde_json::to_value)
        .transpose()?;
    let new = serde_json::to_value(&skill)?;
    let changed = old.as_ref() != Some(&new);
    cfg.skills.insert(name.clone(), skill);
    if changed {
        save(path, cfg)?;
    }
    Ok(Outcome {
        command: if ensuring {
            "skill.ensure"
        } else {
            "skill.add"
        }
        .into(),
        data: serde_json::json!({"name":name,"changed":changed,"skill":cfg.skills[&name]}),
        human: format!(
            "{} skill {name}",
            if ensuring { "ensured" } else { "added" }
        ),
        mutated: changed,
        hint: None,
    })
}

fn execute(cli: Cli) -> Result<Outcome> {
    if cli.json && matches!(&cli.command, Cmd::Tui) {
        bail!("tui is unavailable with --json");
    }
    let path = cli.config.clone().unwrap_or_else(default_config);
    if let Cmd::Init { library } = cli.command {
        // With no explicit --config/SKILLFLEET_CONFIG, co-locate the manifest at
        // the library root so it is committed and versioned with the skills.
        let init_path = cli
            .config
            .clone()
            .unwrap_or_else(|| co_located_config(&library));
        if init_path.exists() {
            bail!("config already exists: {}", init_path.display());
        }
        let cfg = Config {
            schema: 1,
            library,
            endpoints: BTreeMap::new(),
            skills: BTreeMap::new(),
        };
        save(&init_path, &cfg)?;
        let lib_abs = expand(&cfg.library).display().to_string();
        let hint = format!(
            "Make skillfleet location-independent:\n  echo 'export SKILLFLEET_CONFIG={}' >> ~/.bashrc\n  source ~/.bashrc",
            init_path.display()
        );
        return Ok(Outcome {
            command: "init".into(),
            data: serde_json::to_value(&cfg)?,
            human: format!("initialized {} (library {})", init_path.display(), lib_abs),
            mutated: false,
            hint: Some(hint),
        });
    }
    if let Cmd::SelfSkill {
        command: SelfCmd::Update { check },
    } = cli.command
    {
        return self_update(check);
    }
    if let Cmd::Completions { shell } = cli.command {
        return completions_command(shell);
    }
    if let Cmd::Man = cli.command {
        return man_command();
    }
    if let Cmd::SelfSkill {
        command: SelfCmd::Uninstall { yes },
    } = cli.command
    {
        return uninstall_command(yes);
    }
    let mut cfg = load(&path)?;
    let mut outcome = match cli.command {
        Cmd::Init { .. } => unreachable!(),
        Cmd::Completions { .. } | Cmd::Man => unreachable!("handled before config load"),
        Cmd::Endpoint { command } => match command {
            EndpointCmd::Add {
                name,
                path: p,
                no_vacuum,
            } => {
                validate_name(&name)?;
                if cfg.endpoints.contains_key(&name) {
                    bail!("endpoint already exists: {name}");
                }
                cfg.endpoints.insert(
                    name.clone(),
                    Endpoint {
                        path: p,
                        vacuum: !no_vacuum,
                    },
                );
                save(&path, &cfg)?;
                Outcome {
                    command: "endpoint.add".into(),
                    data: serde_json::json!({"name":name,"endpoint":cfg.endpoints[&name]}),
                    human: format!("added endpoint {name}"),
                    mutated: true,
                    hint: None,
                }
            }
            EndpointCmd::Ensure {
                name,
                path: p,
                no_vacuum,
                vacuum,
            } => {
                validate_name(&name)?;
                // Preserve the existing vacuum setting when neither flag is given;
                // --vacuum re-enables, --no-vacuum disables.
                let vacuum_value = if no_vacuum {
                    false
                } else if vacuum {
                    true
                } else {
                    cfg.endpoints.get(&name).map(|e| e.vacuum).unwrap_or(true)
                };
                let old = cfg.endpoints.get(&name);
                let changed = match old {
                    Some(e) => e.path != p || e.vacuum != vacuum_value,
                    None => true,
                };
                cfg.endpoints.insert(
                    name.clone(),
                    Endpoint {
                        path: p,
                        vacuum: vacuum_value,
                    },
                );
                if changed {
                    save(&path, &cfg)?;
                }
                Outcome {
                    command: "endpoint.ensure".into(),
                    data: serde_json::json!({"name":name,"changed":changed,"endpoint":cfg.endpoints[&name]}),
                    human: format!("ensured endpoint {name}"),
                    mutated: changed,
                    hint: None,
                }
            }
            EndpointCmd::Remove { name } => {
                if cfg.skills.values().any(|s| s.targets.contains(&name)) {
                    bail!("endpoint {name} is still targeted by skills");
                }
                cfg.endpoints
                    .remove(&name)
                    .with_context(|| format!("endpoint not found: {name}"))?;
                save(&path, &cfg)?;
                Outcome {
                    command: "endpoint.remove".into(),
                    data: serde_json::json!({"name":name}),
                    human: format!("removed endpoint {name}"),
                    mutated: true,
                    hint: None,
                }
            }
            EndpointCmd::List => Outcome {
                command: "endpoint.list".into(),
                data: serde_json::to_value(&cfg.endpoints)?,
                human: cfg
                    .endpoints
                    .iter()
                    .map(|(n, e)| format!("{n:<16} {}", expand(&e.path).display()))
                    .collect::<Vec<_>>()
                    .join("\n"),
                mutated: false,
                hint: None,
            },
            EndpointCmd::Show { name } => {
                let e = cfg
                    .endpoints
                    .get(&name)
                    .with_context(|| format!("endpoint not found: {name}"))?;
                Outcome {
                    command: "endpoint.show".into(),
                    data: serde_json::json!({"name":name,"endpoint":e}),
                    human: expand(&e.path).display().to_string(),
                    mutated: false,
                    hint: None,
                }
            }
        },
        Cmd::Skill { command } => match command {
            SkillCmd::Add(a) => upsert_skill(&mut cfg, &path, a, false)?,
            SkillCmd::Ensure(a) => upsert_skill(&mut cfg, &path, a, true)?,
            SkillCmd::Remove { name } => {
                cfg.skills
                    .remove(&name)
                    .with_context(|| format!("skill not found: {name}"))?;
                save(&path, &cfg)?;
                Outcome {
                    command: "skill.remove".into(),
                    data: serde_json::json!({"name":name,"source_preserved":true}),
                    human: format!(
                        "removed skill {name}; canonical source preserved; run skillfleet sync"
                    ),
                    mutated: true,
                    hint: None,
                }
            }
            SkillCmd::Unroute { name, from } => {
                ensure_targets(&cfg, &from)?;
                let s = cfg
                    .skills
                    .get_mut(&name)
                    .with_context(|| format!("skill not found: {name}"))?;
                let old = s.targets.clone();
                s.targets.retain(|target| !from.contains(target));
                normalize_targets(&mut s.targets);
                let changed = old != s.targets;
                let targets = s.targets.clone();
                if changed {
                    save(&path, &cfg)?;
                }
                Outcome {
                    command: "skill.unroute".into(),
                    data: serde_json::json!({"name":name,"changed":changed,"targets":targets,"from":from}),
                    human: format!("unrouted {name}; run skillfleet sync"),
                    mutated: changed,
                    hint: None,
                }
            }
            SkillCmd::Delete { name, global } => {
                let data = delete_skill(&mut cfg, &path, &name, global)?;
                Outcome {
                    command: "skill.delete".into(),
                    data,
                    human: format!(
                        "globally deleted skill {name}; links and canonical source removed"
                    ),
                    mutated: true,
                    hint: None,
                }
            }
            SkillCmd::Route { name, to } | SkillCmd::RouteSet { name, to } => {
                ensure_targets(&cfg, &to)?;
                let mut to = to;
                normalize_targets(&mut to);
                let s = cfg
                    .skills
                    .get_mut(&name)
                    .with_context(|| format!("skill not found: {name}"))?;
                let changed = s.targets != to;
                s.targets = to;
                let targets = s.targets.clone();
                if changed {
                    save(&path, &cfg)?;
                }
                Outcome {
                    command: "skill.route.set".into(),
                    data: serde_json::json!({"name":name,"changed":changed,"targets":targets}),
                    human: format!("set routes for {name}"),
                    mutated: changed,
                    hint: None,
                }
            }
            SkillCmd::RouteAdd { name, to } => {
                ensure_targets(&cfg, &to)?;
                let s = cfg
                    .skills
                    .get_mut(&name)
                    .with_context(|| format!("skill not found: {name}"))?;
                let old = s.targets.clone();
                s.targets.extend(to);
                normalize_targets(&mut s.targets);
                let changed = old != s.targets;
                let targets = s.targets.clone();
                if changed {
                    save(&path, &cfg)?;
                }
                Outcome {
                    command: "skill.route.add".into(),
                    data: serde_json::json!({"name":name,"changed":changed,"targets":targets}),
                    human: format!("added routes for {name}"),
                    mutated: changed,
                    hint: None,
                }
            }
            SkillCmd::RouteRemove { name, from } => {
                ensure_targets(&cfg, &from)?;
                let s = cfg
                    .skills
                    .get_mut(&name)
                    .with_context(|| format!("skill not found: {name}"))?;
                let old = s.targets.clone();
                s.targets.retain(|x| !from.contains(x));
                normalize_targets(&mut s.targets);
                let changed = old != s.targets;
                let targets = s.targets.clone();
                if changed {
                    save(&path, &cfg)?;
                }
                Outcome {
                    command: "skill.route.remove".into(),
                    data: serde_json::json!({"name":name,"changed":changed,"targets":targets}),
                    human: format!("removed routes for {name}"),
                    mutated: changed,
                    hint: None,
                }
            }
            SkillCmd::Source {
                name,
                endpoint,
                path: source,
            } => {
                if !cfg.endpoints.contains_key(&endpoint) {
                    bail!("unknown endpoint: {endpoint}");
                }
                cfg.skills
                    .get_mut(&name)
                    .with_context(|| format!("skill not found: {name}"))?
                    .source_overrides
                    .insert(endpoint.clone(), source);
                save(&path, &cfg)?;
                Outcome {
                    command: "skill.source".into(),
                    data: serde_json::json!({"name":name,"endpoint":endpoint}),
                    human: format!("set {name} source override for {endpoint}"),
                    mutated: true,
                    hint: None,
                }
            }
            SkillCmd::List => Outcome {
                command: "skill.list".into(),
                data: serde_json::to_value(&cfg.skills)?,
                human: cfg
                    .skills
                    .iter()
                    .map(|(n, s)| format!("{n:<24} {}", s.targets.join(",")))
                    .collect::<Vec<_>>()
                    .join("\n"),
                mutated: false,
                hint: None,
            },
            SkillCmd::Show { name } => {
                let s = cfg
                    .skills
                    .get(&name)
                    .with_context(|| format!("skill not found: {name}"))?;
                Outcome {
                    command: "skill.show".into(),
                    data: serde_json::json!({"name":name,"skill":s}),
                    human: toml::to_string_pretty(s)?,
                    mutated: false,
                    hint: None,
                }
            }
        },
        Cmd::Plan => {
            let a = plan(&cfg)?;
            let vacuum = vacuum_candidates(&cfg);
            let mut data = plan_document(&a);
            data["vacuum_candidates"] = serde_json::to_value(&vacuum)?;
            let mut human: Vec<String> = a
                .iter()
                .map(|x| {
                    format!(
                        "{:<9} {}:{} -> {}",
                        action_label(x.action),
                        x.endpoint,
                        x.skill,
                        x.destination.display()
                    )
                })
                .collect();
            for c in &vacuum {
                human.push(format!(
                    "{:<9} {}:{} -> {}{}",
                    "vacuum",
                    c.endpoint,
                    c.name,
                    if c.conflict {
                        "not adopted"
                    } else {
                        "sync will adopt into library"
                    },
                    if c.conflict {
                        " (name already declared; plan reports the conflict)"
                    } else {
                        ""
                    }
                ));
            }
            Outcome {
                command: "plan".into(),
                data,
                human: human.join("\n"),
                mutated: false,
                hint: None,
            }
        }
        Cmd::Status => {
            let a = plan(&cfg)?;
            let healthy = a.iter().all(|x| x.action == ActionKind::Ok);
            let vacuum = vacuum_candidates(&cfg);
            let adoptable = vacuum.iter().filter(|c| !c.conflict).count();
            let blocked = vacuum.len() - adoptable;
            let mut pending = String::new();
            if adoptable > 0 {
                pending.push_str(&format!(", {adoptable} pending vacuum adoption(s)"));
            }
            if blocked > 0 {
                pending.push_str(&format!(", {blocked} vacuum name conflict(s)"));
            }
            Outcome {
                command: "status".into(),
                data: serde_json::json!({"config":path,"library":expand(&cfg.library),"endpoints":cfg.endpoints,"skills":cfg.skills,"routes":cfg.skills.iter().map(|(n,s)|serde_json::json!({"skill":n,"targets":s.targets})).collect::<Vec<_>>(),"plan":plan_document(&a),"vacuum_candidates":vacuum,"health":{"ok":healthy}}),
                human: format!(
                    "{} skills, {} endpoints, health: {}{}",
                    cfg.skills.len(),
                    cfg.endpoints.len(),
                    if healthy { "ok" } else { "needs-sync" },
                    pending
                ),
                mutated: false,
                hint: None,
            }
        }
        Cmd::Sync { force } => {
            // Adopt manually added skills from vacuum-enabled endpoints first
            // (mutates cfg + library), then link declared routes.
            let vacuumed = vacuum_endpoints(&mut cfg, &path)?;
            let a = apply(&cfg, force)?;
            let human = if vacuumed.is_empty() {
                format!("sync complete: {} planned entries", a.len())
            } else {
                format!(
                    "sync complete: {} planned entries, {} adopted",
                    a.len(),
                    vacuumed.len()
                )
            };
            Outcome {
                command: "sync".into(),
                data: serde_json::json!({"plan": plan_document(&a), "vacuumed": vacuumed}),
                human,
                mutated: !vacuumed.is_empty(),
                hint: None,
            }
        }
        Cmd::Doctor => {
            let a = plan(&cfg)?;
            let bad: Vec<_> = a.iter().filter(|x| x.action != ActionKind::Ok).collect();
            if !bad.is_empty() {
                bail!(
                    "doctor found {} {}: {} (run 'skillfleet --json plan' for structured detail)",
                    bad.len(),
                    if bad.len() == 1 {
                        "problem"
                    } else {
                        "problems"
                    },
                    bad.iter()
                        .map(|x| format!("{} {}:{}", action_label(x.action), x.endpoint, x.skill))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            Outcome {
                command: "doctor".into(),
                data: serde_json::json!({"ok":true,"skills":cfg.skills.len(),"endpoints":cfg.endpoints.len()}),
                human: format!(
                    "OK: {} skills across {} named endpoints",
                    cfg.skills.len(),
                    cfg.endpoints.len()
                ),
                mutated: false,
                hint: None,
            }
        }
        Cmd::Tui => {
            launch_tui(&path)?;
            Outcome {
                command: "tui".into(),
                data: serde_json::json!({"exited":true}),
                human: "TUI exited".into(),
                mutated: false,
                hint: None,
            }
        }
        Cmd::Update { name, check } => {
            let names: Vec<String> = name.map(|n| vec![n]).unwrap_or_else(|| {
                cfg.skills
                    .iter()
                    .filter(|(_, s)| s.remote.is_some())
                    .map(|(n, _)| n.clone())
                    .collect()
            });
            let mut reports = Vec::new();
            for n in names {
                reports.push(update_remote(
                    &cfg,
                    &n,
                    cfg.skills
                        .get(&n)
                        .with_context(|| format!("skill not found: {n}"))?,
                    check,
                )?);
            }
            let changed = reports.iter().any(|r| r.changed) && !check;
            Outcome {
                command: if check { "update.check" } else { "update" }.into(),
                human: reports
                    .iter()
                    .map(|r| {
                        format!(
                            "{} {} ({})",
                            if check { "checked" } else { "updated" },
                            r.name,
                            if r.changed { "changed" } else { "unchanged" }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
                data: serde_json::to_value(reports)?,
                mutated: changed,
                hint: None,
            }
        }
        Cmd::SelfSkill {
            command: SelfCmd::Install { to },
        } => {
            ensure_targets(&cfg, &to)?;
            let relative = PathBuf::from("skills/skillfleet");
            let directory = expand(&cfg.library).join(&relative);
            fs::create_dir_all(&directory)?;
            fs::write(directory.join("SKILL.md"), SELF_SKILL)?;
            let mut targets = to;
            normalize_targets(&mut targets);
            cfg.skills.insert(
                "skillfleet".into(),
                Skill {
                    source: relative,
                    source_overrides: BTreeMap::new(),
                    targets,
                    remote: None,
                },
            );
            save(&path, &cfg)?;
            Outcome {
                command: "self.install".into(),
                data: serde_json::json!({"name":"skillfleet"}),
                human: "installed bundled skillfleet skill; run skillfleet sync".into(),
                mutated: true,
                hint: None,
            }
        }
        Cmd::SelfSkill {
            command: SelfCmd::Update { .. },
        } => unreachable!("self update is handled before config load"),
        Cmd::SelfSkill {
            command: SelfCmd::Uninstall { .. },
        } => unreachable!("self uninstall is handled before config load"),
    };
    let supports_post_apply = matches!(
        outcome.command.as_str(),
        "endpoint.add"
            | "endpoint.ensure"
            | "endpoint.remove"
            | "skill.add"
            | "skill.ensure"
            | "skill.remove"
            | "skill.unroute"
            | "skill.route.set"
            | "skill.route.add"
            | "skill.route.remove"
            | "skill.source"
            | "update"
            | "self.install"
    );
    if (outcome.mutated || supports_post_apply) && (cli.sync_after || cli.verify) {
        let actions = apply(&cfg, false)?;
        if cli.verify {
            let remaining = plan(&cfg)?;
            if remaining.iter().any(|a| a.action != ActionKind::Ok) {
                bail!("doctor found problems after sync");
            }
        }
        outcome.data = serde_json::json!({"mutation":outcome.data,"sync":plan_document(&actions),"verified":cli.verify});
        outcome.human.push_str(if cli.verify {
            "; synced and verified"
        } else {
            "; synced"
        });
    }
    Ok(outcome)
}

fn launch_tui(config: &Path) -> Result<()> {
    let binary = std::env::var_os("SKILLFLEET_TUI")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe().ok().and_then(|exe| {
                let candidate = exe.with_file_name("skillfleet-tui");
                candidate.is_file().then_some(candidate)
            })
        })
        .unwrap_or_else(|| PathBuf::from("skillfleet-tui"));
    let current = std::env::current_exe().context("locate skillfleet executable")?;
    let status = Command::new(&binary)
        .args(["--config", &config.to_string_lossy()])
        .env("SKILLFLEET_CLI", current)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("launch {}", binary.display()))?;
    if !status.success() {
        bail!("skillfleet-tui exited with {status}");
    }
    Ok(())
}

const RELEASE_API: &str = "https://api.github.com/repos/PhoenixSmith/skillfleet/releases/latest";

fn curl_ok(args: &[&str]) -> Result<String> {
    let out = Command::new("curl")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .context("invoke curl; curl is required for self update")?;
    if !out.status.success() {
        bail!(
            "curl failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn release_target() -> Result<(String, String)> {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        other => bail!("self update is unsupported on {other}"),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => bail!("self update is unsupported on architecture {other}"),
    };
    Ok((os.into(), arch.into()))
}

fn strip_v(tag: &str) -> &str {
    tag.strip_prefix('v').unwrap_or(tag)
}
fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let av: Vec<u64> = a
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .split(['.', '-'])
        .map(|p| p.parse().unwrap_or(0))
        .collect();
    let bv: Vec<u64> = b
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .split(['.', '-'])
        .map(|p| p.parse().unwrap_or(0))
        .collect();
    for i in 0..av.len().max(bv.len()) {
        match av
            .get(i)
            .copied()
            .unwrap_or(0)
            .cmp(&bv.get(i).copied().unwrap_or(0))
        {
            std::cmp::Ordering::Equal => {}
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

fn fetch_latest_release() -> Result<serde_json::Value> {
    let ua = format!("skillfleet/{}", env!("CARGO_PKG_VERSION"));
    let json = curl_ok(&[
        "-fsSL",
        "-A",
        &ua,
        "-H",
        "Accept: application/vnd.github+json",
        RELEASE_API,
    ])?;
    let v: serde_json::Value = serde_json::from_str(&json).context("parse release JSON")?;
    if v["tag_name"].as_str().is_none() {
        bail!("latest release did not expose a tag_name");
    }
    Ok(v)
}

fn asset_url(release: &serde_json::Value, ver: &str, os: &str, arch: &str) -> Result<String> {
    let want = format!("skillfleet-{ver}-{os}-{arch}.tar.gz");
    let Some(assets) = release["assets"].as_array() else {
        bail!("release has no assets");
    };
    for a in assets {
        if a["name"].as_str() == Some(want.as_str()) {
            return a["browser_download_url"]
                .as_str()
                .map(str::to_owned)
                .context("asset missing download url");
        }
    }
    bail!(
        "release has no asset {want}; the release was not built for this platform, or assets are still uploading"
    )
}

fn replace_binary(src: &Path, dst: &Path) -> Result<()> {
    let new = dst.with_extension("update-new");
    fs::copy(src, &new).with_context(|| format!("write {}", new.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&new, fs::Permissions::from_mode(0o755))?;
    }
    fs::rename(&new, dst).with_context(|| format!("install {}", dst.display()))?;
    Ok(())
}

fn sha256_hex(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = fs::read(path)?;
    let digest = Sha256::digest(&bytes);
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

/// Parse an expected sha256 (hex) for `want_file` out of a SHA256SUMS listing.
/// Tolerates both `hash  name` and `hash *name` (GNU coreutils) forms.
fn expected_sha256(sum_text: &str, want_file: &str) -> Result<String> {
    for line in sum_text.lines() {
        let line = line.trim();
        let mut parts = line.split_whitespace();
        let (Some(hash), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        if name == want_file || name.trim_start_matches('*') == want_file {
            return Ok(hash.to_string());
        }
    }
    bail!("SHA256SUMS does not list {want_file}")
}

// Asset files for a release all live in the same GitHub download directory.
fn release_assets_dir(asset_url: &str) -> Result<&str> {
    asset_url
        .rsplit_once('/')
        .map(|(d, _)| d)
        .context("asset URL has no release directory")
}

fn install_release(url: &str, ver: &str) -> Result<()> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("skillfleet-update-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&dir)?;
    let tarball = dir.join(format!("{ver}.tar.gz"));
    let result = (|| {
        let assets_dir = release_assets_dir(url)?;
        let out = Command::new("curl")
            .args(["-fsSL", "-o"])
            .arg(&tarball)
            .arg(url)
            .stdin(Stdio::null())
            .output()?;
        if !out.status.success() {
            bail!(
                "download failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let sum_url = format!("{assets_dir}/SHA256SUMS");
        let sum_text = curl_ok(&["-fsSL", &sum_url])?;
        let want = tarball.file_name().unwrap().to_string_lossy();
        let expected = expected_sha256(&sum_text, &want)?;
        let actual = sha256_hex(&tarball)?;
        if !actual.eq_ignore_ascii_case(&expected) {
            bail!("checksum mismatch for {want}: expected {expected}, got {actual}");
        }
        let status = Command::new("tar")
            .args(["-xzf"])
            .arg(&tarball)
            .arg("-C")
            .arg(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !status.success() {
            bail!("failed to extract release archive");
        }
        let exe = std::env::current_exe().context("locate skillfleet executable")?;
        replace_binary(&dir.join("skillfleet"), &exe)?;
        if dir.join("skillfleet-tui").is_file() {
            replace_binary(
                &dir.join("skillfleet-tui"),
                &exe.with_file_name("skillfleet-tui"),
            )?;
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&dir);
    result
}

fn self_update(check: bool) -> Result<Outcome> {
    let current = env!("CARGO_PKG_VERSION");
    let (os, arch) = release_target()?;
    let release = fetch_latest_release()?;
    let tag = release["tag_name"].as_str().unwrap_or_default();
    let upstream = strip_v(tag);
    let order = version_cmp(current, upstream);
    let data = serde_json::json!({ "current": current, "latest": upstream, "checked": check });
    let command = || {
        if check {
            "self.update.check"
        } else {
            "self.update"
        }
        .to_string()
    };
    if order != std::cmp::Ordering::Less {
        let human = if current == upstream {
            format!("skillfleet is up to date ({current})")
        } else {
            format!("installed {current} is newer than latest release {upstream}")
        };
        return Ok(Outcome {
            command: command(),
            data,
            human,
            mutated: false,
            hint: None,
        });
    }
    if check {
        return Ok(Outcome {
            command: command(),
            data,
            human: format!("update available: {current} -> {upstream}"),
            mutated: false,
            hint: None,
        });
    }
    let url = asset_url(&release, upstream, &os, &arch)?;
    install_release(&url, upstream)?;
    Ok(Outcome {
        command: command(),
        data: serde_json::json!({ "current": current, "latest": upstream, "updated": true }),
        human: format!("updated skillfleet to {upstream}"),
        mutated: true,
        hint: None,
    })
}

fn completions_command(shell: clap_complete::Shell) -> Result<Outcome> {
    use clap::CommandFactory;
    let mut cmd = <Cli as CommandFactory>::command();
    let name = cmd.get_name().to_string();
    let mut buf: Vec<u8> = Vec::new();
    clap_complete::generate(shell, &mut cmd, name, &mut buf);
    Ok(Outcome {
        command: "completions".into(),
        data: serde_json::json!({ "shell": shell.to_string() }),
        human: String::from_utf8(buf).context("generated completion script is not UTF-8")?,
        mutated: false,
        hint: None,
    })
}

fn man_command() -> Result<Outcome> {
    use clap::CommandFactory;
    let cmd = <Cli as CommandFactory>::command();
    let man = clap_mangen::Man::new(cmd);
    let mut buf: Vec<u8> = Vec::new();
    man.render(&mut buf)?;
    Ok(Outcome {
        command: "man".into(),
        data: serde_json::json!({ "format": "roff" }),
        human: String::from_utf8(buf).context("generated man page is not UTF-8")?,
        mutated: false,
        hint: None,
    })
}

fn uninstall_command(yes: bool) -> Result<Outcome> {
    use std::io::IsTerminal;
    let exe = std::env::current_exe().context("locate skillfleet executable")?;
    let tui = exe.with_file_name("skillfleet-tui");
    let mut targets = vec![exe.clone()];
    if tui.is_file() {
        targets.push(tui);
    }
    if !yes {
        if !std::io::stdin().is_terminal() {
            bail!("refusing to wait for input in a non-interactive run; pass --yes to confirm");
        }
        eprintln!("About to remove:");
        for t in &targets {
            eprintln!("  {}", t.display());
        }
        eprint!("Continue? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush().ok();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        let answer = line.trim().to_ascii_lowercase();
        if answer != "y" && answer != "yes" {
            bail!("aborted");
        }
    }
    let mut removed = Vec::new();
    for t in &targets {
        if fs::remove_file(t).is_ok() {
            removed.push(t.to_path_buf());
        }
    }
    Ok(Outcome {
        command: "self.uninstall".into(),
        data: serde_json::json!({
            "removed": removed,
            "remaining": targets.len() - removed.len(),
        }),
        human: if removed.is_empty() {
            "nothing to remove".into()
        } else {
            format!(
                "removed {}",
                removed
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        },
        mutated: !removed.is_empty(),
        hint: None,
    })
}

fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    let wants_json = args.iter().any(|a| a == "--json");
    let cli = match Cli::try_parse_from(args) {
        Ok(c) => c,
        Err(e) => {
            if wants_json {
                println!(
                    "{}",
                    serde_json::json!({"schema_version":1,"ok":false,"error":{"code":"usage_error","message":e.to_string()}})
                );
                std::process::exit(2)
            } else {
                e.exit()
            }
        }
    };
    let json = cli.json;
    match execute(cli) {
        Ok(outcome) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({"schema_version":1,"ok":true,"command":outcome.command,"data":outcome.data})
                );
            } else if !outcome.human.is_empty() {
                println!("{}", outcome.human);
                if let Some(hint) = outcome.hint {
                    println!("\n{hint}");
                }
            }
        }
        Err(error) => {
            let message = format!("{error:#}");
            if json {
                println!(
                    "{}",
                    serde_json::json!({"schema_version":1,"ok":false,"error":{"code":error_code(&message),"message":message}})
                );
            } else {
                eprintln!("Error: {message}");
            }
            std::process::exit(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names() {
        assert!(validate_name("claude-code").is_ok());
        assert!(validate_name("bad/name").is_err());
    }
    #[test]
    fn tilde() {
        unsafe { std::env::set_var("HOME", "/tmp/home") };
        assert_eq!(expand(Path::new("~/x")), PathBuf::from("/tmp/home/x"));
    }

    #[test]
    fn init_co_locates_config_at_library() {
        // clap binds SKILLFLEET_CONFIG as env, so clear it to exercise the
        // no-explicit-config code path deterministically.
        unsafe { std::env::remove_var("SKILLFLEET_CONFIG") };
        let temp = tempfile::tempdir().unwrap();
        let lib = temp.path().join("library");
        std::fs::create_dir_all(&lib).unwrap();
        // No explicit --config: manifest is written beside the library so it is
        // committed and versioned with the skills.
        let mut argv = vec!["skillfleet", "init", "--library", lib.to_str().unwrap()];
        let cli = Cli::try_parse_from(argv.clone()).unwrap();
        let outcome = execute(cli).unwrap();
        assert_eq!(outcome.command, "init");
        assert!(lib.join("skillfleet.toml").is_file());
        // Explicit --config still wins and is honored verbatim.
        let cfg = temp.path().join("elsewhere.toml");
        argv = vec![
            "skillfleet",
            "--config",
            cfg.to_str().unwrap(),
            "init",
            "--library",
            lib.to_str().unwrap(),
        ];
        execute(Cli::try_parse_from(argv).unwrap()).unwrap();
        assert!(cfg.is_file());
    }

    #[test]
    fn find_upward_manifest_honors_repo_local_config() {
        let temp = tempfile::tempdir().unwrap();
        let nested = temp.path().join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        // No manifest anywhere yet -> None.
        assert!(find_upward_manifest(Some(nested.clone())).is_none());
        // Drop a manifest at the repo root; walking up from a nested dir finds it.
        let manifest = temp.path().join("skillfleet.toml");
        std::fs::write(&manifest, "schema = 1\n").unwrap();
        assert_eq!(find_upward_manifest(Some(nested.clone())), Some(manifest));
        // A manifest in a subdirectory shadows the one further up (closest wins).
        let nearer = nested.join("skillfleet.toml");
        std::fs::write(&nearer, "schema = 1\n").unwrap();
        assert_eq!(find_upward_manifest(Some(nested.clone())), Some(nearer));
    }

    #[test]
    fn co_located_config_expands_tilde() {
        unsafe { std::env::set_var("HOME", "/tmp/home") };
        assert_eq!(
            co_located_config(Path::new("~/lib")),
            PathBuf::from("/tmp/home/lib/skillfleet.toml")
        );
        assert_eq!(
            co_located_config(Path::new("/abs/lib")),
            PathBuf::from("/abs/lib/skillfleet.toml")
        );
    }

    #[test]
    fn targets_are_normalized() {
        let mut targets = vec!["pi".into(), "hermes".into(), "pi".into()];
        normalize_targets(&mut targets);
        assert_eq!(targets, vec!["hermes", "pi"]);
    }

    #[test]
    fn plan_reports_missing_source_as_error() {
        let temp = tempfile::tempdir().unwrap();
        let mut cfg = Config {
            schema: 1,
            library: temp.path().join("library"),
            endpoints: BTreeMap::new(),
            skills: BTreeMap::new(),
        };
        cfg.endpoints.insert(
            "agent".into(),
            Endpoint {
                path: temp.path().join("endpoint"),
                vacuum: true,
            },
        );
        cfg.skills.insert(
            "missing".into(),
            Skill {
                source: "skills/missing".into(),
                source_overrides: BTreeMap::new(),
                targets: vec!["agent".into()],
                remote: None,
            },
        );
        let actions = plan(&cfg).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action, ActionKind::Error);
        assert!(!actions[0].destructive);
    }

    #[test]
    fn sync_preflights_missing_sources_before_mutating_endpoints() {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("library");
        let endpoint = temp.path().join("endpoint");
        std::fs::create_dir_all(library.join("skills/a-valid")).unwrap();
        std::fs::write(library.join("skills/a-valid/SKILL.md"), "# valid").unwrap();

        let mut cfg = Config {
            schema: 1,
            library,
            endpoints: BTreeMap::new(),
            skills: BTreeMap::new(),
        };
        cfg.endpoints.insert(
            "agent".into(),
            Endpoint {
                path: endpoint.clone(),
                vacuum: true,
            },
        );
        cfg.skills.insert(
            "a-valid".into(),
            Skill {
                source: "skills/a-valid".into(),
                source_overrides: BTreeMap::new(),
                targets: vec!["agent".into()],
                remote: None,
            },
        );
        // BTreeMap ordering ensures the valid Link action precedes this Error.
        // Before preflight validation, apply() created the valid link and then
        // failed, leaving sync partially applied.
        cfg.skills.insert(
            "z-missing".into(),
            Skill {
                source: "skills/z-missing".into(),
                source_overrides: BTreeMap::new(),
                targets: vec!["agent".into()],
                remote: None,
            },
        );

        let error = apply(&cfg, false).unwrap_err().to_string();
        assert!(error.contains("source missing SKILL.md"));
        assert!(!endpoint.join("a-valid").exists());
        assert!(!endpoint.join("a-valid").is_symlink());
    }

    #[test]
    fn sync_unlinks_managed_link_after_source_is_deleted() {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("library");
        let endpoint = temp.path().join("endpoint");
        let source = library.join("skills/doomed");
        let destination = endpoint.join("doomed");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&endpoint).unwrap();
        std::fs::write(source.join("SKILL.md"), "# doomed").unwrap();
        symlink(&source, &destination).unwrap();
        std::fs::remove_dir_all(&source).unwrap();

        let mut cfg = Config {
            schema: 1,
            library,
            endpoints: BTreeMap::new(),
            skills: BTreeMap::new(),
        };
        cfg.endpoints.insert(
            "agent".into(),
            Endpoint {
                path: endpoint,
                vacuum: true,
            },
        );

        assert!(destination.is_symlink());
        assert!(is_managed_link(&destination, &cfg));
        let actions = apply(&cfg, false).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action, ActionKind::Unlink);
        assert!(!destination.is_symlink());
    }

    #[test]
    fn global_delete_removes_links_declaration_and_canonical_source() {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("library");
        let endpoint = temp.path().join("endpoint");
        let config_path = temp.path().join("skillfleet.toml");
        let source = library.join("skills/doomed");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("SKILL.md"), "# doomed").unwrap();

        let mut cfg = Config {
            schema: 1,
            library,
            endpoints: BTreeMap::new(),
            skills: BTreeMap::new(),
        };
        cfg.endpoints.insert(
            "agent".into(),
            Endpoint {
                path: endpoint.clone(),
                vacuum: false,
            },
        );
        cfg.skills.insert(
            "doomed".into(),
            Skill {
                source: "skills/doomed".into(),
                source_overrides: BTreeMap::new(),
                targets: vec!["agent".into()],
                remote: None,
            },
        );
        save(&config_path, &cfg).unwrap();
        apply(&cfg, false).unwrap();
        assert!(endpoint.join("doomed").is_symlink());

        let report = delete_skill(&mut cfg, &config_path, "doomed", true).unwrap();
        assert_eq!(report["global"], true);
        assert!(!source.exists());
        assert!(!endpoint.join("doomed").is_symlink());
        let persisted = load(&config_path).unwrap();
        assert!(!persisted.skills.contains_key("doomed"));
    }

    #[test]
    fn global_delete_requires_flag_and_refuses_external_source() {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("library");
        let external = temp.path().join("external");
        let config_path = temp.path().join("skillfleet.toml");
        std::fs::create_dir_all(&external).unwrap();
        std::fs::write(external.join("SKILL.md"), "# external").unwrap();
        let mut cfg = Config {
            schema: 1,
            library,
            endpoints: BTreeMap::new(),
            skills: BTreeMap::new(),
        };
        cfg.skills.insert(
            "external".into(),
            Skill {
                source: external.clone(),
                source_overrides: BTreeMap::new(),
                targets: vec![],
                remote: None,
            },
        );
        save(&config_path, &cfg).unwrap();

        assert!(
            delete_skill(&mut cfg, &config_path, "external", false)
                .unwrap_err()
                .to_string()
                .contains("requires --global")
        );
        assert!(
            delete_skill(&mut cfg, &config_path, "external", true)
                .unwrap_err()
                .to_string()
                .contains("outside the canonical library")
        );
        assert!(external.join("SKILL.md").is_file());
        assert!(load(&config_path).unwrap().skills.contains_key("external"));
    }

    #[test]
    fn missing_declared_route_is_self_healing_and_actionable() {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("library");
        let endpoint = temp.path().join("endpoint");
        std::fs::create_dir_all(library.join("skills/heal")).unwrap();
        std::fs::write(library.join("skills/heal/SKILL.md"), "# heal").unwrap();
        let mut cfg = Config {
            schema: 1,
            library,
            endpoints: BTreeMap::new(),
            skills: BTreeMap::new(),
        };
        cfg.endpoints.insert(
            "agent".into(),
            Endpoint {
                path: endpoint.clone(),
                vacuum: false,
            },
        );
        cfg.skills.insert(
            "heal".into(),
            Skill {
                source: "skills/heal".into(),
                source_overrides: BTreeMap::new(),
                targets: vec!["agent".into()],
                remote: None,
            },
        );

        let actions = plan(&cfg).unwrap();
        assert_eq!(actions[0].action, ActionKind::Link);
        assert!(
            actions[0]
                .detail
                .as_deref()
                .unwrap()
                .contains("skill unroute")
        );
        apply(&cfg, false).unwrap();
        assert!(endpoint.join("heal").is_symlink());
        std::fs::remove_file(endpoint.join("heal")).unwrap();
        apply(&cfg, false).unwrap();
        assert!(endpoint.join("heal").is_symlink());
    }

    #[test]
    fn targeted_mutation_sync_does_not_vacuum_unrelated_endpoint_skills() {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("library");
        let endpoint = temp.path().join("endpoint");
        let config_path = temp.path().join("skillfleet.toml");
        std::fs::create_dir_all(library.join("skills/declared")).unwrap();
        std::fs::write(library.join("skills/declared/SKILL.md"), "# declared").unwrap();
        std::fs::create_dir_all(endpoint.join("manual")).unwrap();
        std::fs::write(endpoint.join("manual/SKILL.md"), "# manual").unwrap();
        let mut cfg = Config {
            schema: 1,
            library: library.clone(),
            endpoints: BTreeMap::new(),
            skills: BTreeMap::new(),
        };
        cfg.endpoints.insert(
            "agent".into(),
            Endpoint {
                path: endpoint.clone(),
                vacuum: true,
            },
        );
        save(&config_path, &cfg).unwrap();

        execute(
            Cli::try_parse_from([
                "skillfleet",
                "--config",
                config_path.to_str().unwrap(),
                "--sync",
                "skill",
                "add",
                "declared",
                "--source",
                "skills/declared",
                "--to",
                "agent",
            ])
            .unwrap(),
        )
        .unwrap();

        let persisted = load(&config_path).unwrap();
        assert!(persisted.skills.contains_key("declared"));
        assert!(!persisted.skills.contains_key("manual"));
        assert!(endpoint.join("manual").is_dir());
        assert!(!endpoint.join("manual").is_symlink());
        assert!(!library.join("skills/manual").exists());
    }

    #[test]
    fn add_rejects_duplicate_and_ensure_is_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.toml");
        let library = temp.path().join("library");
        execute(
            Cli::try_parse_from([
                "skillfleet",
                "--config",
                config.to_str().unwrap(),
                "init",
                "--library",
                library.to_str().unwrap(),
            ])
            .unwrap(),
        )
        .unwrap();
        let args = || {
            Cli::try_parse_from([
                "skillfleet",
                "--config",
                config.to_str().unwrap(),
                "endpoint",
                "add",
                "hermes",
                temp.path().to_str().unwrap(),
            ])
            .unwrap()
        };
        execute(args()).unwrap();
        assert!(
            execute(args())
                .unwrap_err()
                .to_string()
                .contains("already exists")
        );
        let ensure = || {
            Cli::try_parse_from([
                "skillfleet",
                "--config",
                config.to_str().unwrap(),
                "endpoint",
                "ensure",
                "hermes",
                temp.path().to_str().unwrap(),
            ])
            .unwrap()
        };
        let outcome = execute(ensure()).unwrap();
        assert_eq!(outcome.data["changed"], false);
    }

    #[test]
    fn plan_document_has_versioned_summary() {
        let doc = plan_document(&[]);
        assert_eq!(doc["schema_version"], 1);
        assert_eq!(doc["summary"]["healthy"], true);
        assert_eq!(doc["summary"]["total"], 0);
    }

    #[test]
    fn version_ordering_is_semverish() {
        assert_eq!(version_cmp("0.1.0", "0.1.0"), std::cmp::Ordering::Equal);
        assert_eq!(version_cmp("0.1.0", "0.2.0"), std::cmp::Ordering::Less);
        assert_eq!(version_cmp("0.2.0", "0.1.0"), std::cmp::Ordering::Greater);
        assert_eq!(version_cmp("0.2.0", "0.10.0"), std::cmp::Ordering::Less);
        assert_eq!(version_cmp("0.10.0", "0.9.1"), std::cmp::Ordering::Greater);
        assert_eq!(version_cmp("1.0.0", "0.9.0"), std::cmp::Ordering::Greater);
    }

    #[test]
    fn tag_prefix_is_stripped() {
        assert_eq!(strip_v("v0.2.0"), "0.2.0");
        assert_eq!(strip_v("0.2.0"), "0.2.0");
    }

    #[test]
    fn checksum_lookup_matches_plain_and_starred_forms() {
        let sums = concat!(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  skillfleet-0.3.0-linux-amd64.tar.gz\n",
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef *skillfleet-0.3.0-darwin-arm64.tar.gz\n",
            "notallchecksums\n",
        );
        let got = expected_sha256(sums, "skillfleet-0.3.0-linux-amd64.tar.gz").unwrap();
        assert_eq!(
            got,
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
        assert_eq!(
            expected_sha256(sums, "skillfleet-0.3.0-darwin-arm64.tar.gz").unwrap(),
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
        );
        assert!(expected_sha256(sums, "skillfleet-0.3.0-windows-amd64.tar.gz").is_err());
    }

    #[test]
    fn legacy_endpoint_config_defaults_vacuum_on() {
        let endpoint: Endpoint = toml::from_str("path = '/tmp/skills'").unwrap();
        assert!(endpoint.vacuum);
    }

    #[test]
    fn vacuum_adopts_skill_and_links_it_back() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("skillfleet.toml");
        let library = temp.path().join("library");
        let endpoint_root = temp.path().join("endpoint");
        let manual = endpoint_root.join("manual-skill");
        fs::create_dir_all(&manual).unwrap();
        fs::write(manual.join("SKILL.md"), "# Manual\n").unwrap();
        fs::write(manual.join("notes.txt"), "kept\n").unwrap();

        let mut cfg = Config {
            schema: 1,
            library: library.clone(),
            endpoints: BTreeMap::from([(
                "agent".into(),
                Endpoint {
                    path: endpoint_root,
                    vacuum: true,
                },
            )]),
            skills: BTreeMap::new(),
        };
        save(&config_path, &cfg).unwrap();
        let reports = vacuum_endpoints(&mut cfg, &config_path).unwrap();

        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].name, "manual-skill");
        assert!(manual.is_symlink());
        assert_eq!(
            fs::read_to_string(manual.join("notes.txt")).unwrap(),
            "kept\n"
        );
        let adopted = cfg.skills.get("manual-skill").unwrap();
        assert_eq!(adopted.targets, vec!["agent"]);
        assert_eq!(adopted.source, PathBuf::from("skills/manual-skill"));
        assert!(library.join("skills/manual-skill/SKILL.md").is_file());
        assert!(
            plan(&cfg)
                .unwrap()
                .iter()
                .all(|action| action.action == ActionKind::Ok)
        );
    }

    #[test]
    fn vacuum_can_be_disabled_per_endpoint() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("skillfleet.toml");
        let manual = temp.path().join("endpoint/local-only");
        fs::create_dir_all(&manual).unwrap();
        fs::write(manual.join("SKILL.md"), "# Local only\n").unwrap();
        let mut cfg = Config {
            schema: 1,
            library: temp.path().join("library"),
            endpoints: BTreeMap::from([(
                "agent".into(),
                Endpoint {
                    path: temp.path().join("endpoint"),
                    vacuum: false,
                },
            )]),
            skills: BTreeMap::new(),
        };
        save(&config_path, &cfg).unwrap();

        assert!(vacuum_endpoints(&mut cfg, &config_path).unwrap().is_empty());
        assert!(manual.is_dir());
        assert!(!manual.is_symlink());
        assert!(cfg.skills.is_empty());
    }

    #[test]
    fn ensure_preserves_vacuum_opt_out() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("skillfleet.toml");
        let run = |args: &[&str]| {
            let mut argv = vec!["skillfleet", "--config", config_path.to_str().unwrap()];
            argv.extend_from_slice(args);
            execute(Cli::try_parse_from(argv).unwrap()).unwrap()
        };
        run(&[
            "init",
            "--library",
            temp.path().join("library").to_str().unwrap(),
        ]);
        let endpoint = temp.path().join("endpoint");
        run(&[
            "endpoint",
            "add",
            "agent",
            endpoint.to_str().unwrap(),
            "--no-vacuum",
        ]);
        run(&["endpoint", "ensure", "agent", endpoint.to_str().unwrap()]);
        let cfg = load(&config_path).unwrap();
        assert!(
            !cfg.endpoints["agent"].vacuum,
            "ensure must preserve opt-out"
        );
        run(&[
            "endpoint",
            "ensure",
            "agent",
            endpoint.to_str().unwrap(),
            "--vacuum",
        ]);
        let cfg = load(&config_path).unwrap();
        assert!(cfg.endpoints["agent"].vacuum, "--vacuum must re-enable");
    }

    #[test]
    fn vacuum_skips_directories_with_invalid_names() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("skillfleet.toml");
        let manual = temp.path().join("endpoint/bad name");
        fs::create_dir_all(&manual).unwrap();
        fs::write(manual.join("SKILL.md"), "# Bad\n").unwrap();
        let mut cfg = Config {
            schema: 1,
            library: temp.path().join("library"),
            endpoints: BTreeMap::from([(
                "agent".into(),
                Endpoint {
                    path: temp.path().join("endpoint"),
                    vacuum: true,
                },
            )]),
            skills: BTreeMap::new(),
        };
        save(&config_path, &cfg).unwrap();

        assert!(vacuum_endpoints(&mut cfg, &config_path).unwrap().is_empty());
        assert!(manual.is_dir());
        assert!(!manual.is_symlink());
    }

    #[test]
    fn vacuum_fails_closed_on_declared_name_collision() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("skillfleet.toml");
        let manual = temp.path().join("endpoint/collision");
        fs::create_dir_all(&manual).unwrap();
        fs::write(manual.join("SKILL.md"), "# Collision\n").unwrap();
        let mut cfg = Config {
            schema: 1,
            library: temp.path().join("library"),
            endpoints: BTreeMap::from([(
                "agent".into(),
                Endpoint {
                    path: temp.path().join("endpoint"),
                    vacuum: true,
                },
            )]),
            skills: BTreeMap::from([(
                "collision".into(),
                Skill {
                    source: "skills/collision".into(),
                    source_overrides: BTreeMap::new(),
                    targets: Vec::new(),
                    remote: None,
                },
            )]),
        };
        save(&config_path, &cfg).unwrap();

        let reports = vacuum_endpoints(&mut cfg, &config_path).unwrap();
        assert!(reports.is_empty(), "declared name must not be adopted");
        assert!(manual.is_dir());
        assert!(!manual.is_symlink());
        assert_eq!(cfg.skills.len(), 1, "no new skill registered");
    }

    #[test]
    fn vacuum_candidates_flags_conflicts_and_skips_disabled() {
        let temp = tempfile::tempdir().unwrap();
        for (ep, skill) in [("on", "fresh"), ("on", "collision"), ("off", "hidden")] {
            let d = temp.path().join(ep).join(skill);
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("SKILL.md"), "# x\n").unwrap();
        }
        let cfg = Config {
            schema: 1,
            library: temp.path().join("library"),
            endpoints: BTreeMap::from([
                (
                    "on".into(),
                    Endpoint {
                        path: temp.path().join("on"),
                        vacuum: true,
                    },
                ),
                (
                    "off".into(),
                    Endpoint {
                        path: temp.path().join("off"),
                        vacuum: false,
                    },
                ),
            ]),
            skills: BTreeMap::from([(
                "collision".into(),
                Skill {
                    source: "skills/collision".into(),
                    source_overrides: BTreeMap::new(),
                    targets: Vec::new(),
                    remote: None,
                },
            )]),
        };
        let found = vacuum_candidates(&cfg);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().any(|c| c.name == "fresh" && !c.conflict));
        assert!(found.iter().any(|c| c.name == "collision" && c.conflict));
    }

    #[test]
    fn vacuum_never_adopts_force_backups() {
        let temp = tempfile::tempdir().unwrap();
        let backup = temp.path().join("endpoint/cleanup.skillfleet-backup");
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("SKILL.md"), "# old\n").unwrap();
        let cfg = Config {
            schema: 1,
            library: temp.path().join("library"),
            endpoints: BTreeMap::from([(
                "agent".into(),
                Endpoint {
                    path: temp.path().join("endpoint"),
                    vacuum: true,
                },
            )]),
            skills: BTreeMap::new(),
        };
        assert!(vacuum_candidates(&cfg).is_empty());
    }

    #[test]
    fn plan_and_status_report_vacuum_candidates() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("skillfleet.toml");
        let run = |args: &[&str]| {
            let mut argv = vec!["skillfleet", "--config", config_path.to_str().unwrap()];
            argv.extend_from_slice(args);
            execute(Cli::try_parse_from(argv).unwrap()).unwrap()
        };
        run(&[
            "init",
            "--library",
            temp.path().join("library").to_str().unwrap(),
        ]);
        let endpoint = temp.path().join("endpoint");
        let manual = endpoint.join("manual-skill");
        fs::create_dir_all(&manual).unwrap();
        fs::write(manual.join("SKILL.md"), "# m\n").unwrap();
        run(&["endpoint", "add", "agent", endpoint.to_str().unwrap()]);

        let plan_out = run(&["plan"]);
        let names: Vec<_> = plan_out.data["vacuum_candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, ["manual-skill"]);
        assert!(plan_out.human.contains("vacuum"));

        let status_out = run(&["status"]);
        assert_eq!(
            status_out.data["vacuum_candidates"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(status_out.human.contains("pending vacuum"));
    }

    #[test]
    fn error_messages_are_specific() {
        assert_eq!(
            error_code("doctor found 1 problem: conflict claude:x"),
            "verification_failed"
        );
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("skillfleet.toml");
        let run = |args: &[&str]| {
            let mut argv = vec!["skillfleet", "--config", config_path.to_str().unwrap()];
            argv.extend_from_slice(args);
            execute(Cli::try_parse_from(argv).unwrap())
        };
        let missing = run(&["status"]).unwrap_err().to_string();
        assert!(missing.contains("skillfleet init"), "{missing}");
        run(&[
            "init",
            "--library",
            temp.path().join("library").to_str().unwrap(),
        ])
        .unwrap();
        let not_found = format!("{:#}", run(&["skill", "show", "ghost"]).unwrap_err());
        assert!(not_found.contains("skill not found: ghost"), "{not_found}");
    }
}
