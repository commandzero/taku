use super::discovery::{RemoteDiscovery, discover_remote};
use super::{
    FetchResult, Observation, ObservationFile, binding, cache_path, hash,
    is_transformation_conflict, load_observation, save_observation,
};
use crate::application::load_installed;
use crate::canonical::{
    InventoryEntry, Selection, canonical_bytes, list_inventory_with_resolved, pointer_string,
};
use crate::lifecycle::{DeletionMarker, deletion_markers};
use crate::provider::{SecretFields, resolve_auth};
use crate::resolution::ResolvedApplication;
use crate::transport::{
    INTERNAL_GUARD_POINTER, OperationInput, RemoteResult, execute_retry_safe, remove_pointer,
};
use crate::{ApplicationDefinition, Operation, ResourceType, SCHEMA_VERSION, TargetConfig};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde_json::Value;
use std::collections::{BTreeMap, btree_map::Entry};
use std::path::{Path, PathBuf};

fn requires_pull_after_configuration_change(
    previous: Option<&Observation>,
    hints: &crate::hints::HintResolution,
    current_binding: &str,
    structural_binding_changed: bool,
) -> bool {
    previous.is_some_and(|observation| {
        observation.requires_pull
            || structural_binding_changed
            || observation.hint_binding.as_ref().map_or_else(
                || hints.target_bytes.is_some() || hints.resource_bytes.is_some(),
                |binding| binding != current_binding,
            )
    })
}

pub fn fetch(
    root: &Path,
    selection: &Selection,
    cli_provider: &BTreeMap<String, String>,
) -> Result<Vec<FetchResult>> {
    let mut discovery = discover_remote(root, selection, cli_provider)?;
    let inventory =
        list_inventory_with_resolved(&discovery.root, selection, Some(&discovery.resolved))?;
    discovery.persist_baselines()?;
    let mut cache = FetchCache::default();
    let mut reports = Vec::new();
    for item in &inventory {
        reports.push(cache.observe(&mut discovery, Input::Resource(item), cli_provider)?);
    }
    for (path, marker) in deletion_markers(&discovery.root, selection)? {
        reports.push(cache.observe(&mut discovery, Input::Marker(&path, &marker), cli_provider)?);
    }
    cache.save()?;
    reports.sort_by(|a, b| {
        (&a.environment, &a.target, &a.resource_type, &a.id).cmp(&(
            &b.environment,
            &b.target,
            &b.resource_type,
            &b.id,
        ))
    });
    Ok(reports)
}

enum Input<'a> {
    Resource(&'a InventoryEntry),
    Marker(&'a Path, &'a DeletionMarker),
}

struct Identity<'a> {
    target: &'a str,
    namespace: Option<&'a str>,
    resource_type: &'a str,
    id: &'a str,
}

impl Input<'_> {
    fn identity(&self) -> Identity<'_> {
        match self {
            Self::Resource(item) => Identity {
                target: &item.target,
                namespace: item.namespace.as_deref(),
                resource_type: &item.resource_type,
                id: &item.id,
            },
            Self::Marker(_, marker) => Identity {
                target: &marker.target,
                namespace: marker.namespace.as_deref(),
                resource_type: &marker.resource_type,
                id: &marker.id,
            },
        }
    }

    fn context(&self) -> std::borrow::Cow<'_, Value> {
        match self {
            Self::Resource(item) => std::borrow::Cow::Borrowed(&item.value),
            Self::Marker(_, marker) => std::borrow::Cow::Owned(Value::Object(
                marker
                    .parameters
                    .iter()
                    .map(|(key, value)| (key.clone(), Value::String(value.clone())))
                    .collect(),
            )),
        }
    }

    fn local_state(
        &self,
        root: &Path,
        previous: Option<&Observation>,
        requires_pull: bool,
    ) -> Result<(String, Option<Value>, String)> {
        match self {
            Self::Resource(item) => {
                let current_hash = hash(&canonical_bytes(&item.value)?);
                let (hash, value) = match previous.filter(|_| requires_pull) {
                    Some(previous) => (previous.local_hash.clone(), previous.local_value.clone()),
                    None => (current_hash, Some(item.value.clone())),
                };
                Ok((hash, value, item.path.clone()))
            }
            Self::Marker(path, marker) => Ok((
                hash(marker.guard.as_bytes()),
                None,
                path.strip_prefix(root).unwrap().display().to_string(),
            )),
        }
    }

    fn read_error(&self) -> &'static str {
        match self {
            Self::Resource(_) => "selected Resource Type has no Read Operation",
            Self::Marker(..) => "Deletion Marker Resource Type has no Read Operation",
        }
    }
}

#[derive(Default)]
struct FetchCache {
    groups: BTreeMap<PathBuf, ObservationGroup>,
}

struct ObservationGroup {
    file: ObservationFile,
    binding: String,
    binding_changed: bool,
}

impl FetchCache {
    fn observe(
        &mut self,
        discovery: &mut RemoteDiscovery,
        input: Input<'_>,
        cli_provider: &BTreeMap<String, String>,
    ) -> Result<FetchResult> {
        let identity = input.identity();
        let target =
            &discovery.project.environments[&discovery.environment].targets[identity.target];
        let app = load_installed(&discovery.root, &target.application)?;
        discovery.resolved_target(identity.target, &app, cli_provider)?;
        let hints = crate::hints::resolve(
            &discovery.root,
            &discovery.project,
            &discovery.environment,
            identity.target,
            identity.namespace,
            identity.resource_type,
        )?;
        let hint_binding = hash(&hints.binding_material(&discovery.root)?);
        let read = RemoteRead::for_input(discovery, &input, &app, cli_provider)?;
        let remote = match read.execute(&input, hints.track) {
            Ok(remote) => remote,
            Err(error) if is_transformation_conflict(&error) => {
                return Ok(identity.report(&discovery.environment, "transformation_conflict"));
            }
            Err(error) => return Err(error),
        };
        let (mut value, outcome) = read_outcome(remote, identity.id)?;
        let group = self.group(discovery, &identity, &app)?;
        let previous = group.file.resources.get(identity.id);
        let requires_pull = requires_pull_after_configuration_change(
            previous,
            &hints,
            &hint_binding,
            group.binding_changed,
        );
        group.refresh(&discovery.resolved[identity.target]);
        let previous = group.file.resources.get(identity.id);
        let (local_hash, local_value, path) =
            input.local_state(&discovery.root, previous, requires_pull)?;
        let guard = extract_guard(&mut value)?;
        group.file.resources.insert(
            identity.id.to_owned(),
            Observation {
                local_hash,
                local_value,
                path,
                present: value.is_some(),
                value,
                guard,
                requires_pull,
                hint_binding: Some(hint_binding),
            },
        );
        Ok(identity.report(&discovery.environment, outcome))
    }

    fn group(
        &mut self,
        discovery: &RemoteDiscovery,
        identity: &Identity<'_>,
        app: &ApplicationDefinition,
    ) -> Result<&mut ObservationGroup> {
        let path = cache_path(
            &discovery.root,
            &discovery.environment,
            identity.target,
            identity.namespace,
            identity.resource_type,
        );
        Ok(match self.groups.entry(path) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let group = ObservationGroup::load(entry.key(), discovery, identity, app)?;
                entry.insert(group)
            }
        })
    }

    fn save(self) -> Result<()> {
        for (path, group) in self.groups {
            save_observation(&path, &group.file)?;
        }
        Ok(())
    }
}

impl ObservationGroup {
    fn load(
        path: &Path,
        discovery: &RemoteDiscovery,
        identity: &Identity<'_>,
        app: &ApplicationDefinition,
    ) -> Result<Self> {
        let binding = binding(
            &discovery.root,
            &discovery.project,
            &discovery.environment,
            identity.target,
            app,
            identity.namespace,
            identity.resource_type,
        )?;
        let resolved = &discovery.resolved[identity.target];
        let file = if path.exists() {
            load_observation(path)?
        } else {
            ObservationFile {
                schema_version: SCHEMA_VERSION,
                observed_at: Utc::now(),
                binding: binding.clone(),
                application_version: resolved.application_version.clone(),
                catalog_version: resolved.catalog_version.clone(),
                definitions: BTreeMap::new(),
                resources: BTreeMap::new(),
            }
        };
        Ok(Self {
            binding_changed: file.binding != binding,
            file,
            binding,
        })
    }

    fn refresh(&mut self, resolved: &ResolvedApplication) {
        self.file.observed_at = Utc::now();
        self.file.binding.clone_from(&self.binding);
        self.file
            .application_version
            .clone_from(&resolved.application_version);
        self.file
            .catalog_version
            .clone_from(&resolved.catalog_version);
        self.file.definitions.clone_from(&resolved.selected);
    }
}

impl Identity<'_> {
    fn report(&self, environment: &str, outcome: &str) -> FetchResult {
        FetchResult {
            environment: environment.to_owned(),
            target: self.target.to_owned(),
            namespace: self.namespace.map(str::to_owned),
            resource_type: self.resource_type.to_owned(),
            id: self.id.to_owned(),
            outcome: outcome.into(),
        }
    }
}

struct RemoteRead<'a> {
    target: &'a TargetConfig,
    app: &'a ApplicationDefinition,
    resource_type: &'a ResourceType,
    operation: &'a Operation,
    auth: SecretFields,
}

impl<'a> RemoteRead<'a> {
    fn for_input(
        discovery: &'a RemoteDiscovery,
        input: &Input<'_>,
        app: &'a ApplicationDefinition,
        cli_provider: &BTreeMap<String, String>,
    ) -> Result<Self> {
        let identity = input.identity();
        let target =
            &discovery.project.environments[&discovery.environment].targets[identity.target];
        let resource_type =
            &discovery.resolved[identity.target].resource_types[identity.resource_type];
        let operation = resource_type
            .operations
            .read
            .as_ref()
            .context(input.read_error())?;
        let auth = resolve_auth(
            &discovery.root,
            &discovery.project,
            &discovery.environment,
            target,
            cli_provider,
        )?;
        Ok(Self {
            target,
            app,
            resource_type,
            operation,
            auth,
        })
    }

    fn execute(&self, input: &Input<'_>, metadata_track: bool) -> Result<RemoteResult> {
        let identity = input.identity();
        if matches!(input, Input::Resource(item) if item.pending) {
            return Ok(RemoteResult::NotFound);
        }
        let context = input.context();
        execute_retry_safe(
            self.target,
            self.app,
            self.resource_type,
            self.operation,
            OperationInput {
                namespace: identity.namespace,
                id: Some(identity.id),
                context: Some(&context),
                metadata_track,
                ..OperationInput::default()
            },
            &self.auth,
        )
    }
}

fn read_outcome(remote: RemoteResult, id: &str) -> Result<(Option<Value>, &'static str)> {
    match remote {
        RemoteResult::Success(mut values) if values.len() == 1 => Ok((values.pop(), "observed")),
        RemoteResult::NotFound => Ok((None, "absent")),
        RemoteResult::Conflict => bail!("Read Operation reported a conflict for {id}"),
        RemoteResult::Retryable | RemoteResult::Uncertain => {
            bail!("Read Operation failed transiently for {id}")
        }
        RemoteResult::Failure(message) => bail!("Read Operation failed for {id}: {message}"),
        RemoteResult::Success(_) => bail!("One Read Operation returned multiple Resources"),
    }
}

fn extract_guard(value: &mut Option<Value>) -> Result<Option<String>> {
    let Some(value) = value else { return Ok(None) };
    let guard = pointer_string(value, INTERNAL_GUARD_POINTER)
        .unwrap_or_else(|| hash(&canonical_bytes(value).unwrap_or_default()));
    remove_pointer(value, INTERNAL_GUARD_POINTER)?;
    Ok(Some(guard))
}
