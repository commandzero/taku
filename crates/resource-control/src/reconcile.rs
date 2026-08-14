use crate::application::load_installed;
use crate::canonical::{Selection, canonical_bytes, list_inventory, owned_value, write_resource};
use crate::observe::{binding, cache_path, hash, load_observation, save_observation};
use crate::project::current_environment;
use crate::variants::{baseline_from, baseline_path, save_baseline};
use crate::{MissingPolicy, git_root, load_project};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
pub struct Comparison {
    pub environment: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
    pub state: String,
    pub observed_age_seconds: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct DiffEntry {
    pub environment: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
    pub state: String,
    pub desired: Value,
    pub observed: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PullResult {
    pub environment: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
    pub outcome: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PushResult {
    pub environment: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
    pub outcome: String,
    pub git_revision: String,
    pub safety: String,
}

pub fn compare(root: &Path, selection: &Selection) -> Result<Vec<Comparison>> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let environment = current_environment(&root, &project, selection.environment.as_deref())?;
    let mut result = Vec::new();
    let mut applications = BTreeMap::new();
    let mut bindings = BTreeMap::new();
    let mut observations = BTreeMap::new();
    for item in list_inventory(&root, selection)? {
        let target = &project.environments[&environment].targets[&item.target];
        if !applications.contains_key(&item.target) {
            applications.insert(
                item.target.clone(),
                load_installed(&root, &target.application)?,
            );
        }
        let application = &applications[&item.target];
        let resource_type = &application.target_profile.resource_types[&item.resource_type];
        let observation_path = cache_path(
            &root,
            &environment,
            &item.target,
            item.namespace.as_deref(),
            &item.resource_type,
        );
        if !observations.contains_key(&observation_path) {
            observations.insert(
                observation_path.clone(),
                load_observation(&observation_path)?,
            );
        }
        if !bindings.contains_key(&item.target) {
            bindings.insert(
                item.target.clone(),
                binding(&root, &project, &environment, &item.target, application)?,
            );
        }
        let observation = &observations[&observation_path];
        if observation.binding != bindings[&item.target] {
            bail!("Observed State is structurally invalid; run `taku fetch`");
        }
        let resource = observation
            .resources
            .get(&item.id)
            .context("selected Resource is absent from Observed State; run `taku fetch`")?;
        let desired_hash = hash(&canonical_bytes(&item.value)?);
        let observed_value = resource
            .value
            .as_ref()
            .map(|value| owned_value(value, &item.value, resource_type.mutation_mode));
        let state = if !resource.present {
            "presence_conflict"
        } else if observed_value
            .as_ref()
            .is_some_and(|value| hash(&canonical_bytes(value).unwrap_or_default()) == desired_hash)
        {
            "in_sync"
        } else {
            "drift"
        };
        result.push(Comparison {
            environment: environment.clone(),
            target: item.target,
            namespace: item.namespace,
            resource_type: item.resource_type,
            id: item.id,
            state: state.into(),
            observed_age_seconds: (Utc::now() - observation.observed_at).num_seconds().max(0),
        });
    }
    Ok(result)
}

pub fn diff(root: &Path, selection: &Selection) -> Result<Vec<DiffEntry>> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let environment = current_environment(&root, &project, selection.environment.as_deref())?;
    let mut result = Vec::new();
    let mut applications = BTreeMap::new();
    let mut bindings = BTreeMap::new();
    let mut observations: BTreeMap<PathBuf, crate::observe::ObservationFile> = BTreeMap::new();
    for item in list_inventory(&root, selection)? {
        let target = &project.environments[&environment].targets[&item.target];
        if !applications.contains_key(&item.target) {
            applications.insert(
                item.target.clone(),
                load_installed(&root, &target.application)?,
            );
        }
        let application = &applications[&item.target];
        let resource_type = &application.target_profile.resource_types[&item.resource_type];
        let observation_path = cache_path(
            &root,
            &environment,
            &item.target,
            item.namespace.as_deref(),
            &item.resource_type,
        );
        if !observations.contains_key(&observation_path) {
            observations.insert(
                observation_path.clone(),
                load_observation(&observation_path)?,
            );
        }
        if !bindings.contains_key(&item.target) {
            bindings.insert(
                item.target.clone(),
                binding(&root, &project, &environment, &item.target, application)?,
            );
        }
        let observation = &observations[&observation_path];
        if observation.binding != bindings[&item.target] {
            bail!("Observed State is structurally invalid; run `taku fetch`");
        }
        let resource = observation
            .resources
            .get(&item.id)
            .context("selected Resource is absent from Observed State")?;
        let observed = resource
            .value
            .as_ref()
            .map(|value| owned_value(value, &item.value, resource_type.mutation_mode));
        let state = if !resource.present {
            "presence_conflict"
        } else if observed.as_ref() == Some(&item.value) {
            "in_sync"
        } else {
            "drift"
        };
        if state != "in_sync" {
            result.push(DiffEntry {
                environment: environment.clone(),
                target: item.target,
                namespace: item.namespace,
                resource_type: item.resource_type,
                id: item.id,
                state: state.into(),
                desired: item.value,
                observed,
            });
        }
    }
    Ok(result)
}

pub fn pull(
    root: &Path,
    selection: &Selection,
    missing: Option<MissingPolicy>,
) -> Result<Vec<PullResult>> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let environment = current_environment(&root, &project, selection.environment.as_deref())?;
    let mut result = Vec::new();
    let mut observations = std::collections::BTreeMap::new();
    let mut mutations: Vec<(std::path::PathBuf, Option<Value>)> = Vec::new();
    let mut baselines = std::collections::BTreeMap::new();
    let mut has_conflict = false;
    for item in list_inventory(&root, selection)? {
        let target = &project.environments[&environment].targets[&item.target];
        let application = load_installed(&root, &target.application)?;
        let resource_type = &application.target_profile.resource_types[&item.resource_type];
        let path = cache_path(
            &root,
            &environment,
            &item.target,
            item.namespace.as_deref(),
            &item.resource_type,
        );
        if !observations.contains_key(&path) {
            observations.insert(path.clone(), load_observation(&path)?);
        }
        let observation = observations.get_mut(&path).unwrap();
        if observation.binding
            != binding(&root, &project, &environment, &item.target, &application)?
        {
            bail!("Observed State is structurally invalid; run `taku fetch`");
        }
        let resource = observation
            .resources
            .get_mut(&item.id)
            .context("selected Resource is absent from Observed State")?;
        let current_hash = hash(&canonical_bytes(&item.value)?);
        let outcome = if !resource.present {
            match missing
                .or(resource_type.missing.pull)
                .unwrap_or(MissingPolicy::Conflict)
            {
                MissingPolicy::Delete => {
                    mutations.push((root.join(&item.path), None));
                    "deleted"
                }
                MissingPolicy::Conflict => {
                    has_conflict = true;
                    "presence_conflict"
                }
                MissingPolicy::Restore => bail!("restore is not valid for Pull"),
            }
        } else {
            let remote = resource.value.as_ref().unwrap();
            if let Some(base) = resource.local_value.as_ref() {
                let remote_at_base = owned_value(remote, base, resource_type.mutation_mode);
                let remote_at_current =
                    owned_value(remote, &item.value, resource_type.mutation_mode);
                let local_changed = canonical_bytes(&item.value)? != canonical_bytes(base)?;
                let remote_changed = canonical_bytes(&remote_at_base)? != canonical_bytes(base)?;
                if !local_changed && !remote_changed {
                    "unchanged"
                } else if local_changed && !remote_changed {
                    "local_only"
                } else if !local_changed {
                    let remote_hash = hash(&canonical_bytes(&remote_at_current)?);
                    mutations.push((root.join(&item.path), Some(remote_at_current.clone())));
                    resource.local_hash = remote_hash;
                    resource.local_value = Some(remote_at_current);
                    "pulled"
                } else if canonical_bytes(&remote_at_current)? == canonical_bytes(&item.value)? {
                    resource.local_hash = current_hash;
                    resource.local_value = Some(item.value.clone());
                    "unchanged"
                } else {
                    has_conflict = true;
                    "pull_conflict"
                }
            } else {
                let observed = owned_value(remote, &item.value, resource_type.mutation_mode);
                let remote_hash = hash(&canonical_bytes(&observed)?);
                if current_hash == remote_hash {
                    "unchanged"
                } else if current_hash == resource.local_hash {
                    mutations.push((root.join(&item.path), Some(observed)));
                    resource.local_hash = remote_hash;
                    "pulled"
                } else if remote_hash == resource.local_hash {
                    "local_only"
                } else {
                    has_conflict = true;
                    "pull_conflict"
                }
            }
        };
        if !matches!(outcome, "pull_conflict" | "presence_conflict") {
            baselines.insert(
                baseline_path(&root, &environment, &item.target),
                baseline_from(observation.facts.clone(), observation.variants.clone()),
            );
        }
        result.push(PullResult {
            environment: environment.clone(),
            target: item.target,
            namespace: item.namespace,
            resource_type: item.resource_type,
            id: item.id,
            outcome: outcome.into(),
        });
    }
    if has_conflict {
        for report in &mut result {
            if matches!(report.outcome.as_str(), "pulled" | "deleted") {
                report.outcome = "blocked_by_conflict".into();
            }
        }
        return Ok(result);
    }
    apply_pull_mutations(&mutations)?;
    for (path, observation) in observations {
        save_observation(&path, &observation)?;
    }
    for (path, baseline) in baselines {
        save_baseline(&path, &baseline)?;
    }
    Ok(result)
}

fn apply_pull_mutations(mutations: &[(std::path::PathBuf, Option<Value>)]) -> Result<()> {
    let mut snapshots = Vec::new();
    for (path, value) in mutations {
        let before = if path.exists() {
            Some(fs::read(path)?)
        } else {
            None
        };
        let applied = match value {
            Some(value) => write_resource(path, value),
            None => fs::remove_file(path).map_err(Into::into),
        };
        if let Err(error) = applied {
            for (applied_path, contents) in snapshots.iter().rev() {
                match contents {
                    Some(contents) => {
                        let _ = fs::write(applied_path, contents);
                    }
                    None => {
                        let _ = fs::remove_file(applied_path);
                    }
                }
            }
            return Err(error);
        }
        snapshots.push((path.clone(), before));
    }
    Ok(())
}
