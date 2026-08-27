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
    #[arg(long, global = true, env = "SKILLFLEET_CONFIG")]
    config: Option<PathBuf>,
    #[arg(long, global = true)]
    json: bool,
    #[arg(long = "sync", global = true)]
    sync_after: bool,
    #[arg(long, global = true)]
    verify: bool,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Init {
        #[arg(long)]
        library: PathBuf,
    },
    Endpoint {
        #[command(subcommand)]
        command: EndpointCmd,
    },
    Skill {
        #[command(subcommand)]
        command: SkillCmd,
    },
    Plan,
    Status,
    Sync {
        #[arg(long)]
        force: bool,
    },
    Doctor,
    /// Open the interactive Bubble Tea routing inspector.
    Tui,
    Update {
        name: Option<String>,
        #[arg(long)]
        check: bool,
    },
    #[command(name = "self")]
    SelfSkill {
        #[command(subcommand)]
        command: SelfCmd,
    },
}

#[derive(Subcommand)]
enum SelfCmd {
    Install {
        #[arg(long, num_args = 1..)]
        to: Vec<String>,
    },
    /// Check for and install the latest released binary.
    Update {
        /// Report whether an update is available without installing.
        #[arg(long)]
        check: bool,
    },
}

#[derive(Subcommand)]
enum EndpointCmd {
    Add { name: String, path: PathBuf },
    Ensure { name: String, path: PathBuf },
    Remove { name: String },
    List,
    Show { name: String },
}

#[derive(Subcommand)]
enum SkillCmd {
    Add(SkillAdd),
    Ensure(SkillAdd),
    Remove {
        name: String,
    },
    Route {
        name: String,
        #[arg(long, num_args=0..)]
        to: Vec<String>,
    },
    RouteSet {
        name: String,
        #[arg(long, num_args=0..)]
        to: Vec<String>,
    },
    RouteAdd {
        name: String,
        #[arg(long, num_args=1..)]
        to: Vec<String>,
    },
    RouteRemove {
        name: String,
        #[arg(long, num_args=1..)]
        from: Vec<String>,
    },
    Source {
        name: String,
        #[arg(long = "for")]
        endpoint: String,
        path: PathBuf,
    },
    List,
    Show {
        name: String,
    },
}

#[derive(Args)]
struct SkillAdd {
    name: String,
    #[arg(long, conflicts_with = "git")]
    source: Option<PathBuf>,
    #[arg(long, conflicts_with = "source")]
    git: Option<String>,
    #[arg(long)]
    subdir: Option<PathBuf>,
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
    dirs_home().join(".config/skillfleet/skillfleet.toml")
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
    if message.contains("already exists") {
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
    } else if message.contains("doctor found") {
        "verification_failed"
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
    fs::canonicalize(path)
        .ok()
        .is_some_and(|p| p.starts_with(expand(&cfg.library)))
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
                    detail: None,
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
fn apply(cfg: &Config, force: bool) -> Result<Vec<Action>> {
    let actions = plan(cfg)?;
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
    })
}

fn execute(cli: Cli) -> Result<Outcome> {
    if cli.json && matches!(&cli.command, Cmd::Tui) {
        bail!("tui is unavailable with --json");
    }
    let path = cli.config.clone().unwrap_or_else(default_config);
    if let Cmd::Init { library } = cli.command {
        if path.exists() {
            bail!("config already exists: {}", path.display());
        }
        let cfg = Config {
            schema: 1,
            library,
            endpoints: BTreeMap::new(),
            skills: BTreeMap::new(),
        };
        save(&path, &cfg)?;
        return Ok(Outcome {
            command: "init".into(),
            data: serde_json::to_value(&cfg)?,
            human: format!("initialized {}", path.display()),
            mutated: false,
        });
    }
    if let Cmd::SelfSkill {
        command: SelfCmd::Update { check },
    } = cli.command
    {
        return self_update(check);
    }
    let mut cfg = load(&path)?;
    let mut outcome = match cli.command {
        Cmd::Init { .. } => unreachable!(),
        Cmd::Endpoint { command } => match command {
            EndpointCmd::Add { name, path: p } => {
                validate_name(&name)?;
                if cfg.endpoints.contains_key(&name) {
                    bail!("endpoint already exists: {name}");
                }
                cfg.endpoints.insert(name.clone(), Endpoint { path: p });
                save(&path, &cfg)?;
                Outcome {
                    command: "endpoint.add".into(),
                    data: serde_json::json!({"name":name,"endpoint":cfg.endpoints[&name]}),
                    human: format!("added endpoint {name}"),
                    mutated: true,
                }
            }
            EndpointCmd::Ensure { name, path: p } => {
                validate_name(&name)?;
                let changed = cfg.endpoints.get(&name).is_none_or(|e| e.path != p);
                cfg.endpoints.insert(name.clone(), Endpoint { path: p });
                if changed {
                    save(&path, &cfg)?;
                }
                Outcome {
                    command: "endpoint.ensure".into(),
                    data: serde_json::json!({"name":name,"changed":changed,"endpoint":cfg.endpoints[&name]}),
                    human: format!("ensured endpoint {name}"),
                    mutated: changed,
                }
            }
            EndpointCmd::Remove { name } => {
                if cfg.skills.values().any(|s| s.targets.contains(&name)) {
                    bail!("endpoint {name} is still targeted by skills");
                }
                cfg.endpoints.remove(&name).context("endpoint not found")?;
                save(&path, &cfg)?;
                Outcome {
                    command: "endpoint.remove".into(),
                    data: serde_json::json!({"name":name}),
                    human: format!("removed endpoint {name}"),
                    mutated: true,
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
            },
            EndpointCmd::Show { name } => {
                let e = cfg.endpoints.get(&name).context("endpoint not found")?;
                Outcome {
                    command: "endpoint.show".into(),
                    data: serde_json::json!({"name":name,"endpoint":e}),
                    human: expand(&e.path).display().to_string(),
                    mutated: false,
                }
            }
        },
        Cmd::Skill { command } => match command {
            SkillCmd::Add(a) => upsert_skill(&mut cfg, &path, a, false)?,
            SkillCmd::Ensure(a) => upsert_skill(&mut cfg, &path, a, true)?,
            SkillCmd::Remove { name } => {
                cfg.skills.remove(&name).context("skill not found")?;
                save(&path, &cfg)?;
                Outcome {
                    command: "skill.remove".into(),
                    data: serde_json::json!({"name":name}),
                    human: format!("removed skill {name}; run skillfleet sync"),
                    mutated: true,
                }
            }
            SkillCmd::Route { name, to } | SkillCmd::RouteSet { name, to } => {
                ensure_targets(&cfg, &to)?;
                let mut to = to;
                normalize_targets(&mut to);
                let s = cfg.skills.get_mut(&name).context("skill not found")?;
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
                }
            }
            SkillCmd::RouteAdd { name, to } => {
                ensure_targets(&cfg, &to)?;
                let s = cfg.skills.get_mut(&name).context("skill not found")?;
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
                }
            }
            SkillCmd::RouteRemove { name, from } => {
                ensure_targets(&cfg, &from)?;
                let s = cfg.skills.get_mut(&name).context("skill not found")?;
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
                    .context("skill not found")?
                    .source_overrides
                    .insert(endpoint.clone(), source);
                save(&path, &cfg)?;
                Outcome {
                    command: "skill.source".into(),
                    data: serde_json::json!({"name":name,"endpoint":endpoint}),
                    human: format!("set {name} source override for {endpoint}"),
                    mutated: true,
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
            },
            SkillCmd::Show { name } => {
                let s = cfg.skills.get(&name).context("skill not found")?;
                Outcome {
                    command: "skill.show".into(),
                    data: serde_json::json!({"name":name,"skill":s}),
                    human: toml::to_string_pretty(s)?,
                    mutated: false,
                }
            }
        },
        Cmd::Plan => {
            let a = plan(&cfg)?;
            Outcome {
                command: "plan".into(),
                data: plan_document(&a),
                human: a
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
                    .collect::<Vec<_>>()
                    .join("\n"),
                mutated: false,
            }
        }
        Cmd::Status => {
            let a = plan(&cfg)?;
            let healthy = a.iter().all(|x| x.action == ActionKind::Ok);
            Outcome {
                command: "status".into(),
                data: serde_json::json!({"config":path,"library":expand(&cfg.library),"endpoints":cfg.endpoints,"skills":cfg.skills,"routes":cfg.skills.iter().map(|(n,s)|serde_json::json!({"skill":n,"targets":s.targets})).collect::<Vec<_>>(),"plan":plan_document(&a),"health":{"ok":healthy}}),
                human: format!(
                    "{} skills, {} endpoints, health: {}",
                    cfg.skills.len(),
                    cfg.endpoints.len(),
                    if healthy { "ok" } else { "needs-sync" }
                ),
                mutated: false,
            }
        }
        Cmd::Sync { force } => {
            let a = apply(&cfg, force)?;
            Outcome {
                command: "sync".into(),
                data: plan_document(&a),
                human: format!("sync complete: {} planned entries", a.len()),
                mutated: false,
            }
        }
        Cmd::Doctor => {
            let a = plan(&cfg)?;
            let bad: Vec<_> = a.iter().filter(|x| x.action != ActionKind::Ok).collect();
            if !bad.is_empty() {
                bail!(
                    "doctor found {} problems: {}",
                    bad.len(),
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
            }
        }
        Cmd::Tui => {
            launch_tui(&path)?;
            Outcome {
                command: "tui".into(),
                data: serde_json::json!({"exited":true}),
                human: "TUI exited".into(),
                mutated: false,
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
                    cfg.skills.get(&n).context("skill not found")?,
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
            }
        }
        Cmd::SelfSkill {
            command: SelfCmd::Update { .. },
        } => unreachable!("self update is handled before config load"),
    };
    let supports_post_apply = matches!(
        outcome.command.as_str(),
        "endpoint.add"
            | "endpoint.ensure"
            | "endpoint.remove"
            | "skill.add"
            | "skill.ensure"
            | "skill.remove"
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
        });
    }
    if check {
        return Ok(Outcome {
            command: command(),
            data,
            human: format!("update available: {current} -> {upstream}"),
            mutated: false,
        });
    }
    let url = asset_url(&release, upstream, &os, &arch)?;
    install_release(&url, upstream)?;
    Ok(Outcome {
        command: command(),
        data: serde_json::json!({ "current": current, "latest": upstream, "updated": true }),
        human: format!("updated skillfleet to {upstream}"),
        mutated: true,
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
                println!("{}", outcome.human)
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
}
