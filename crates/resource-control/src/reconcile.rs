mod pull;

pub use pull::pull;

use crate::application::load_installed;
use crate::canonical::{InventoryEntry, Selection, canonical_bytes, list_inventory, owned_value};
use crate::observe::{ObservationFile, binding, cache_path, hash, load_observation};
use crate::project::current_environment;
use crate::resolution::{baseline_path, from_baseline, load_baseline};
use crate::{ApplicationDefinition, Project, git_root, load_project};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, btree_map::Entry};
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
    let mut view = ObservedInventory::new(root, selection)?;
    let mut result = Vec::new();
    for item in list_inventory(&view.root, selection)? {
        let observed = view.read(
            &item,
            "selected Resource is absent from Observed State; run `taku fetch`",
        )?;
        let desired_hash = hash(&canonical_bytes(&item.value)?);
        let equal = observed
            .value
            .as_ref()
            .is_some_and(|value| hash(&canonical_bytes(value).unwrap_or_default()) == desired_hash);
        result.push(Comparison {
            environment: view.environment.clone(),
            target: item.target,
            namespace: item.namespace,
            resource_type: item.resource_type,
            id: item.id,
            state: observed.state(equal).into(),
            observed_age_seconds: (Utc::now() - observed.observed_at).num_seconds().max(0),
        });
    }
    Ok(result)
}

pub fn diff(root: &Path, selection: &Selection) -> Result<Vec<DiffEntry>> {
    let mut view = ObservedInventory::new(root, selection)?;
    let mut result = Vec::new();
    for item in list_inventory(&view.root, selection)? {
        let observed = view.read(&item, "selected Resource is absent from Observed State")?;
        let state = observed.state(observed.value.as_ref() == Some(&item.value));
        if state != "in_sync" {
            result.push(DiffEntry {
                environment: view.environment.clone(),
                target: item.target,
                namespace: item.namespace,
                resource_type: item.resource_type,
                id: item.id,
                state: state.into(),
                desired: item.value,
                observed: observed.value,
            });
        }
    }
    Ok(result)
}

struct ObservedItem {
    present: bool,
    value: Option<Value>,
    observed_at: chrono::DateTime<Utc>,
}

impl ObservedItem {
    fn state(&self, equal: bool) -> &'static str {
        match (self.present, equal) {
            (false, _) => "presence_conflict",
            (true, true) => "in_sync",
            (true, false) => "drift",
        }
    }
}

struct ObservedInventory {
    root: PathBuf,
    project: Project,
    environment: String,
    applications: BTreeMap<String, ApplicationDefinition>,
    bindings: BTreeMap<PathBuf, String>,
    observations: BTreeMap<PathBuf, ObservationFile>,
}

impl ObservedInventory {
    fn new(root: &Path, selection: &Selection) -> Result<Self> {
        let root = git_root(root)?;
        let project = load_project(&root)?;
        let environment = current_environment(&root, &project, selection.environment.as_deref())?;
        Ok(Self {
            root,
            project,
            environment,
            applications: BTreeMap::new(),
            bindings: BTreeMap::new(),
            observations: BTreeMap::new(),
        })
    }

    fn read(&mut self, item: &InventoryEntry, missing: &'static str) -> Result<ObservedItem> {
        let target = &self.project.environments[&self.environment].targets[&item.target];
        let application = match self.applications.entry(item.target.clone()) {
            Entry::Vacant(entry) => entry.insert(load_installed(&self.root, &target.application)?),
            Entry::Occupied(entry) => entry.into_mut(),
        };
        let baseline = load_baseline(&baseline_path(&self.root, &self.environment, &item.target))?;
        let resolved = from_baseline(application, target, &baseline)?;
        let resource_type = &resolved.resource_types[&item.resource_type];
        let namespace = item.namespace.as_deref();
        let path = cache_path(
            &self.root,
            &self.environment,
            &item.target,
            namespace,
            &item.resource_type,
        );
        let observation = observation_at(&mut self.observations, &path)?;
        let expected = match self.bindings.entry(path) {
            Entry::Vacant(entry) => entry.insert(binding(
                &self.root,
                &self.project,
                &self.environment,
                &item.target,
                application,
                namespace,
                &item.resource_type,
            )?),
            Entry::Occupied(entry) => entry.into_mut(),
        };
        check_binding(observation, expected)?;
        let resource = observation.resources.get(&item.id).context(missing)?;
        Ok(ObservedItem {
            present: resource.present,
            value: resource
                .value
                .as_ref()
                .map(|value| owned_value(value, &item.value, resource_type.mutation_mode)),
            observed_at: observation.observed_at,
        })
    }
}

fn observation_at<'a>(
    observations: &'a mut BTreeMap<PathBuf, ObservationFile>,
    path: &Path,
) -> Result<&'a mut ObservationFile> {
    Ok(match observations.entry(path.to_owned()) {
        Entry::Vacant(entry) => entry.insert(load_observation(path)?),
        Entry::Occupied(entry) => entry.into_mut(),
    })
}

fn check_binding(observation: &ObservationFile, expected: &str) -> Result<()> {
    if observation.binding != expected {
        bail!("Observed State is structurally invalid; run `taku fetch`");
    }
    Ok(())
}
