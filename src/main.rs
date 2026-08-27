use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    process::Command,
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
    Sync {
        #[arg(long)]
        force: bool,
    },
    Doctor,
    Update {
        name: Option<String>,
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
}

#[derive(Subcommand)]
enum EndpointCmd {
    Add { name: String, path: PathBuf },
    Remove { name: String },
    List,
    Show { name: String },
}

#[derive(Subcommand)]
enum SkillCmd {
    Add(SkillAdd),
    Remove {
        name: String,
    },
    Route {
        name: String,
        #[arg(long, num_args=0..)]
        to: Vec<String>,
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
#[derive(Debug, Serialize)]
struct Action {
    action: &'static str,
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
fn emit<T: Serialize>(json: bool, value: &T, human: impl FnOnce()) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        human();
    }
    Ok(())
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
            let Some(ep) = cfg.endpoints.get(target) else {
                out.push(Action {
                    action: "error",
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
                    action: "link",
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
                            action: "ok",
                            skill: name.clone(),
                            endpoint: target.clone(),
                            destination: dst,
                            source: Some(src.clone()),
                            detail: None,
                        });
                    } else {
                        out.push(Action {
                            action: "relink",
                            skill: name.clone(),
                            endpoint: target.clone(),
                            destination: dst,
                            source: Some(src.clone()),
                            detail: None,
                        });
                    }
                }
                Ok(_) => out.push(Action {
                    action: "conflict",
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
                        action: "unlink",
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
            "link" => {
                let src = a.source.as_ref().unwrap();
                if !src.join("SKILL.md").is_file() {
                    bail!("{} missing SKILL.md", src.display())
                }
                fs::create_dir_all(a.destination.parent().unwrap())?;
                symlink(src, &a.destination)?;
            }
            "relink" => {
                fs::remove_file(&a.destination)?;
                symlink(a.source.as_ref().unwrap(), &a.destination)?;
            }
            "unlink" => fs::remove_file(&a.destination)?,
            "conflict" if force => {
                let b = backup_path(&a.destination);
                fs::rename(&a.destination, &b)?;
                symlink(a.source.as_ref().unwrap(), &a.destination)?;
            }
            "conflict" => bail!(
                "conflict at {}; rerun sync --force to back it up",
                a.destination.display()
            ),
            "error" => bail!("{}", a.detail.as_deref().unwrap_or("plan error")),
            _ => {}
        }
    }
    Ok(actions)
}
fn clone_remote(cfg: &Config, name: &str, skill: &Skill) -> Result<()> {
    let remote = skill.remote.as_ref().context("skill is not remote")?;
    let vendor = expand(&cfg.library).join("vendor");
    fs::create_dir_all(&vendor)?;
    let checkout = vendor.join(format!(".{name}-checkout"));
    if checkout.exists() {
        fs::remove_dir_all(&checkout)?;
    }
    let status = Command::new("git")
        .args(["clone", "--depth", "1", &remote.git])
        .arg(&checkout)
        .status()?;
    if !status.success() {
        bail!("git clone failed for {name}")
    }
    let selected = remote
        .subdir
        .as_ref()
        .map(|p| checkout.join(p))
        .unwrap_or(checkout.clone());
    if !selected.join("SKILL.md").is_file() {
        bail!("remote {name} has no SKILL.md at {}", selected.display())
    }
    let dst = source_path(cfg, skill, None);
    let tmp = dst.with_extension("update-tmp");
    if tmp.exists() {
        fs::remove_dir_all(&tmp)?;
    }
    fs::create_dir_all(tmp.parent().unwrap())?;
    fs::rename(&selected, &tmp)?;
    if dst.exists() {
        fs::remove_dir_all(&dst)?;
    }
    fs::rename(tmp, dst)?;
    if checkout.exists() {
        fs::remove_dir_all(checkout)?;
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let path = cli.config.unwrap_or_else(default_config);
    if let Cmd::Init { library } = cli.command {
        if path.exists() {
            bail!("config already exists: {}", path.display())
        }
        let cfg = Config {
            schema: 1,
            library,
            endpoints: BTreeMap::new(),
            skills: BTreeMap::new(),
        };
        save(&path, &cfg)?;
        return emit(cli.json, &cfg, || {
            println!("initialized {}", path.display())
        });
    }
    let mut cfg = load(&path)?;
    match cli.command {
        Cmd::Init { .. } => unreachable!(),
        Cmd::Endpoint { command } => match command {
            EndpointCmd::Add { name, path: p } => {
                validate_name(&name)?;
                cfg.endpoints.insert(name.clone(), Endpoint { path: p });
                save(&path, &cfg)?;
                emit(cli.json, &cfg.endpoints[&name], || {
                    println!("added endpoint {name}")
                })?;
            }
            EndpointCmd::Remove { name } => {
                if cfg.skills.values().any(|s| s.targets.contains(&name)) {
                    bail!("endpoint {name} is still targeted by skills")
                };
                cfg.endpoints.remove(&name).context("endpoint not found")?;
                save(&path, &cfg)?;
                println!("removed endpoint {name}");
            }
            EndpointCmd::List => emit(cli.json, &cfg.endpoints, || {
                for (n, e) in &cfg.endpoints {
                    println!("{n:<16} {}", expand(&e.path).display())
                }
            })?,
            EndpointCmd::Show { name } => {
                let e = cfg.endpoints.get(&name).context("endpoint not found")?;
                emit(cli.json, e, || println!("{}", expand(&e.path).display()))?;
            }
        },
        Cmd::Skill { command } => match command {
            SkillCmd::Add(a) => {
                validate_name(&a.name)?;
                ensure_targets(&cfg, &a.to)?;
                let skill = if let Some(url) = a.git {
                    Skill {
                        source: PathBuf::from(format!("vendor/{}", a.name)),
                        source_overrides: BTreeMap::new(),
                        targets: a.to,
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
                        targets: a.to,
                        remote: None,
                    }
                };
                cfg.skills.insert(a.name.clone(), skill);
                save(&path, &cfg)?;
                println!("added skill {}", a.name);
            }
            SkillCmd::Remove { name } => {
                cfg.skills.remove(&name).context("skill not found")?;
                save(&path, &cfg)?;
                println!("removed skill {name}; run skillfleet sync")
            }
            SkillCmd::Route { name, to } => {
                ensure_targets(&cfg, &to)?;
                cfg.skills
                    .get_mut(&name)
                    .context("skill not found")?
                    .targets = to;
                save(&path, &cfg)?;
                println!("routed {name}");
            }
            SkillCmd::Source {
                name,
                endpoint,
                path: source,
            } => {
                if !cfg.endpoints.contains_key(&endpoint) {
                    bail!("unknown endpoint: {endpoint}")
                }
                cfg.skills
                    .get_mut(&name)
                    .context("skill not found")?
                    .source_overrides
                    .insert(endpoint.clone(), source);
                save(&path, &cfg)?;
                println!("set {name} source override for {endpoint}");
            }
            SkillCmd::List => emit(cli.json, &cfg.skills, || {
                for (n, s) in &cfg.skills {
                    println!("{n:<24} {}", s.targets.join(","))
                }
            })?,
            SkillCmd::Show { name } => {
                let s = cfg.skills.get(&name).context("skill not found")?;
                emit(cli.json, s, || {
                    println!("{}", toml::to_string_pretty(s).unwrap())
                })?;
            }
        },
        Cmd::Plan => {
            let a = plan(&cfg)?;
            emit(cli.json, &a, || {
                for x in &a {
                    println!(
                        "{:<9} {}:{} -> {}",
                        x.action,
                        x.endpoint,
                        x.skill,
                        x.destination.display()
                    )
                }
            })?;
        }
        Cmd::Sync { force } => {
            let a = apply(&cfg, force)?;
            emit(cli.json, &a, || {
                println!("sync complete: {} planned entries", a.len())
            })?;
        }
        Cmd::Doctor => {
            let a = plan(&cfg)?;
            let bad: Vec<_> = a.iter().filter(|x| x.action != "ok").collect();
            if !bad.is_empty() {
                emit(cli.json, &bad, || {
                    for x in &bad {
                        println!("{} {}:{}", x.action, x.endpoint, x.skill)
                    }
                })?;
                bail!("doctor found {} problems", bad.len())
            }
            emit(
                cli.json,
                &serde_json::json!({"ok":true,"skills":cfg.skills.len(),"endpoints":cfg.endpoints.len()}),
                || {
                    println!(
                        "OK: {} skills across {} named endpoints",
                        cfg.skills.len(),
                        cfg.endpoints.len()
                    )
                },
            )?;
        }
        Cmd::Update { name } => {
            let names: Vec<String> = name.map(|n| vec![n]).unwrap_or_else(|| {
                cfg.skills
                    .iter()
                    .filter(|(_, s)| s.remote.is_some())
                    .map(|(n, _)| n.clone())
                    .collect()
            });
            for n in names {
                let s = cfg.skills.get(&n).context("skill not found")?;
                clone_remote(&cfg, &n, s)?;
                println!("updated {n}")
            }
        }
        Cmd::SelfSkill { command } => match command {
            SelfCmd::Install { to } => {
                ensure_targets(&cfg, &to)?;
                let relative = PathBuf::from("skills/skillfleet");
                let directory = expand(&cfg.library).join(&relative);
                fs::create_dir_all(&directory)?;
                fs::write(directory.join("SKILL.md"), SELF_SKILL)?;
                cfg.skills.insert(
                    "skillfleet".into(),
                    Skill {
                        source: relative,
                        source_overrides: BTreeMap::new(),
                        targets: to,
                        remote: None,
                    },
                );
                save(&path, &cfg)?;
                println!("installed bundled skillfleet skill; run skillfleet sync");
            }
        },
    }
    Ok(())
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
}
