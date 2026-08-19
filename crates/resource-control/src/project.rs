use crate::{
    EnvironmentConfig, InitResult, Project, RepositoryLayout, SCHEMA_VERSION, TargetConfig,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextFile {
    schema_version: u32,
    environment: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct TargetListing {
    pub environment: String,
    pub name: String,
    pub application: String,
    pub url: String,
}

pub fn git_root(path: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(path)
        .output()
        .context("failed to inspect Git worktree")?;
    if !output.status.success() {
        bail!("not inside a Git worktree");
    }
    let root =
        String::from_utf8(output.stdout).context("Git returned a non-UTF-8 worktree path")?;
    fs::canonicalize(root.trim()).context("failed to resolve Git worktree root")
}

pub fn initialize(
    path: &Path,
    layout: RepositoryLayout,
    environment_names: Vec<String>,
) -> Result<InitResult> {
    let requested = fs::canonicalize(path).context("failed to resolve project path")?;
    let root = git_root(&requested)?;
    if requested != root {
        bail!(
            "Taku must be initialized at the exact Git worktree root: {}",
            root.display()
        );
    }
    if environment_names.is_empty() {
        bail!("at least one Environment is required");
    }
    if layout == RepositoryLayout::Single && environment_names.len() != 1 {
        bail!("Single layout requires exactly one Environment");
    }
    let taku_dir = root.join(".taku");
    let project_path = taku_dir.join("project.yaml");
    if project_path.exists() {
        bail!("Project is already initialized");
    }
    let mut environments = BTreeMap::new();
    for name in &environment_names {
        if name.is_empty()
            || environments
                .insert(name.clone(), EnvironmentConfig::default())
                .is_some()
        {
            bail!("Environment names must be non-empty and unique");
        }
    }
    fs::create_dir_all(&taku_dir).context("failed to create .taku directory")?;
    let project = Project {
        schema_version: SCHEMA_VERSION,
        layout,
        environments,
        application_source: None,
        push: Default::default(),
        max_requests: 4,
    };
    save_project(&root, &project)?;
    fs::write(
        taku_dir.join(".gitignore"),
        "/cache/\n/context.yaml\n/journals/\n",
    )
    .context("failed to write Taku ignore rules")?;
    Ok(InitResult {
        project: root.display().to_string(),
        layout,
        environments: environment_names,
    })
}

pub fn load_project(root: &Path) -> Result<Project> {
    let root = git_root(root)?;
    let text = fs::read_to_string(root.join(".taku/project.yaml")).context("not a Taku Project")?;
    let project: Project = serde_yaml::from_str(&text).context("invalid Project metadata")?;
    if project.schema_version != SCHEMA_VERSION {
        bail!(
            "unsupported Project schema version {}",
            project.schema_version
        );
    }
    if project.max_requests == 0 {
        bail!("max_requests must be greater than zero");
    }
    validate_environment_graph(&project)?;
    Ok(project)
}

pub fn save_project(root: &Path, project: &Project) -> Result<()> {
    let yaml = serde_yaml::to_string(project).context("failed to serialize Project")?;
    fs::write(root.join(".taku/project.yaml"), yaml).context("failed to write Project metadata")
}

pub fn current_environment(
    root: &Path,
    project: &Project,
    selected: Option<&str>,
) -> Result<String> {
    if let Some(name) = selected {
        if !project.environments.contains_key(name) {
            bail!("unknown Environment {name}");
        }
        return Ok(name.to_owned());
    }
    if project.layout == RepositoryLayout::Single {
        return Ok(project.environments.keys().next().unwrap().clone());
    }
    let path = root.join(".taku/context.yaml");
    let value: ContextFile = serde_yaml::from_str(
        &fs::read_to_string(path)
            .context("no current Environment; select one with `taku context set <environment>`")?,
    )?;
    if value.schema_version != SCHEMA_VERSION {
        bail!(
            "unsupported Context schema version {}",
            value.schema_version
        );
    }
    let name = value.environment;
    if !project.environments.contains_key(&name) {
        bail!("Context selects unknown Environment {name}");
    }
    Ok(name)
}

pub fn save_context(root: &Path, environment: &str) -> Result<String> {
    let project = load_project(root)?;
    if !project.environments.contains_key(environment) {
        bail!("unknown Environment {environment}");
    }
    fs::write(
        root.join(".taku/context.yaml"),
        format!("schema_version: 1\nenvironment: {environment}\n"),
    )?;
    Ok(environment.to_owned())
}

pub fn list_targets(root: &Path, environment: Option<&str>) -> Result<Vec<TargetListing>> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let environment = current_environment(&root, &project, environment)?;
    let targets = &project.environments[&environment].targets;
    Ok(targets
        .iter()
        .map(|(name, target)| TargetListing {
            environment: environment.clone(),
            name: name.clone(),
            application: target.application.clone(),
            url: target.url.clone(),
        })
        .collect())
}

pub fn add_target(
    root: &Path,
    environment: Option<&str>,
    application: &str,
    name: &str,
    url: &str,
) -> Result<TargetConfig> {
    let root = git_root(root)?;
    let mut project = load_project(&root)?;
    let env_name = current_environment(&root, &project, environment)?;
    if !root
        .join(".taku/applications")
        .join(application)
        .join("application.yaml")
        .is_file()
    {
        bail!("Application {application} is not installed");
    }
    let env = project.environments.get_mut(&env_name).unwrap();
    if env.targets.contains_key(name) {
        bail!("Target {name} already exists in Environment {env_name}");
    }
    let target = TargetConfig {
        application: application.into(),
        url: url.trim_end_matches('/').into(),
        from: None,
        auth: None,
        headers: BTreeMap::new(),
        sensitive_fields: BTreeMap::new(),
    };
    env.targets.insert(name.into(), target.clone());
    save_project(&root, &project)?;
    Ok(target)
}

pub fn rename_target(root: &Path, environment: Option<&str>, old: &str, new: &str) -> Result<()> {
    let root = git_root(root)?;
    let mut project = load_project(&root)?;
    let env_name = current_environment(&root, &project, environment)?;
    let env = project.environments.get_mut(&env_name).unwrap();
    if env.targets.contains_key(new) {
        bail!("Target {new} already exists in Environment {env_name}");
    }
    let target = env
        .targets
        .remove(old)
        .with_context(|| format!("unknown Target {old}"))?;
    env.targets.insert(new.into(), target);
    let prefix = if project.layout == RepositoryLayout::Multi {
        root.join(&env_name)
    } else {
        root.clone()
    };
    let old_path = prefix.join(old);
    if old_path.exists() {
        fs::rename(&old_path, prefix.join(new)).context("failed to rename Target Resource tree")?;
    }
    for disposable in [
        root.join(".taku/cache").join(&env_name).join(old),
        root.join(".taku/journals").join(&env_name).join(old),
        root.join(".taku/baselines")
            .join(&env_name)
            .join(format!("{old}.yaml")),
        root.join(".taku/journals")
            .join(&env_name)
            .join("push.yaml"),
    ] {
        if disposable.exists() {
            if disposable.is_dir() {
                fs::remove_dir_all(disposable)?;
            } else {
                fs::remove_file(disposable)?;
            }
        }
    }
    save_project(&root, &project)
}

fn validate_environment_graph(project: &Project) -> Result<()> {
    for start in project.environments.keys() {
        let mut seen = std::collections::BTreeSet::new();
        let mut next = Some(start.as_str());
        while let Some(name) = next {
            if !seen.insert(name.to_owned()) {
                bail!("Environment promotion mapping contains a cycle at {name}");
            }
            next = project
                .environments
                .get(name)
                .and_then(|e| e.from.as_deref());
            if let Some(parent) = next
                && !project.environments.contains_key(parent)
            {
                bail!("Environment {name} maps from unknown Environment {parent}");
            }
        }
    }
    Ok(())
}
