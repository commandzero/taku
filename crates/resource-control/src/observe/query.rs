mod pagination;

use super::discovery::{RemoteDiscovery, discover_remote};
use super::{RemoteEntry, RemoteResourceType};
use crate::application::load_installed;
use crate::canonical::{Selection, canonical_id, display_name_value, list_inventory_with_resolved};
use crate::project::current_environment;
use crate::provider::{SecretFields, resolve_auth};
use crate::resolution::discover;
use crate::transport::{
    INTERNAL_GUARD_POINTER, OperationInput, RemoteResult, execute_retry_safe, remove_pointer,
};
use crate::{
    ApplicationDefinition, Cardinality, Operation, ResourceType, TargetConfig, git_root,
    load_project,
};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub fn remote_list(
    root: &Path,
    selection: &Selection,
    untracked_only: bool,
    cli_provider: &BTreeMap<String, String>,
) -> Result<Vec<RemoteEntry>> {
    ListQuery::new(root, selection, untracked_only, cli_provider).run(true)
}

/// Query remote Resources without persisting Baselines or any other Project state.
pub fn remote_list_read_only(
    root: &Path,
    selection: &Selection,
    untracked_only: bool,
    cli_provider: &BTreeMap<String, String>,
) -> Result<Vec<RemoteEntry>> {
    ListQuery::new(root, selection, untracked_only, cli_provider).run(false)
}

struct ListQuery<'a> {
    root: &'a Path,
    selection: &'a Selection,
    untracked_only: bool,
    cli_provider: &'a BTreeMap<String, String>,
}

impl<'a> ListQuery<'a> {
    fn new(
        root: &'a Path,
        selection: &'a Selection,
        untracked_only: bool,
        cli_provider: &'a BTreeMap<String, String>,
    ) -> Self {
        Self {
            root,
            selection,
            untracked_only,
            cli_provider,
        }
    }

    fn run(&self, persist_baselines: bool) -> Result<Vec<RemoteEntry>> {
        let discovery = discover_remote(self.root, self.selection, self.cli_provider)?;
        if persist_baselines {
            discovery.persist_baselines()?;
        }
        remote_query(
            &discovery,
            self.selection,
            self.untracked_only,
            self.cli_provider,
        )
    }
}

/// Discover the effective listable Resource Types for one Target without persistence.
pub fn remote_resource_types_read_only(
    root: &Path,
    environment: Option<&str>,
    target_name: &str,
    cli_provider: &BTreeMap<String, String>,
) -> Result<Vec<RemoteResourceType>> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    crate::hints::validate_project_placement(&root, &project)?;
    let environment = current_environment(&root, &project, environment)?;
    let target = project.environments[&environment]
        .targets
        .get(target_name)
        .with_context(|| format!("unknown Target {target_name}"))?;
    let app = load_installed(&root, &target.application)?;
    crate::hints::validate_application_tree(&root, &project, &environment, target_name, &app)?;
    let auth = resolve_auth(&root, &project, &environment, target, cli_provider)?;
    Ok(discover(&app, target, &auth)?
        .resource_types
        .into_iter()
        .filter(|(_, resource_type)| {
            resource_type
                .operations
                .list
                .as_ref()
                .is_some_and(|operation| operation.cardinality == crate::Cardinality::Many)
        })
        .map(|(name, resource_type)| RemoteResourceType {
            name,
            namespaced: resource_type.namespaced,
        })
        .collect())
}

type LocalIds = BTreeSet<(String, Option<String>, String, String)>;

fn remote_query(
    discovery: &RemoteDiscovery,
    selection: &Selection,
    untracked_only: bool,
    cli_provider: &BTreeMap<String, String>,
) -> Result<Vec<RemoteEntry>> {
    let local_ids =
        list_inventory_with_resolved(&discovery.root, selection, Some(&discovery.resolved))?
            .into_iter()
            .map(|item| (item.target, item.namespace, item.resource_type, item.id))
            .collect();
    let mut out = Vec::new();
    for (name, target) in &discovery.project.environments[&discovery.environment].targets {
        if selection.targets.is_empty() || selection.targets.contains(name) {
            let query = TargetQuery {
                discovery,
                name,
                target,
                selection,
                local_ids: &local_ids,
                untracked_only,
            };
            query.collect(cli_provider, &mut out)?;
        }
    }
    out.sort_by(|a, b| {
        (&a.target, &a.namespace, &a.resource_type, &a.id).cmp(&(
            &b.target,
            &b.namespace,
            &b.resource_type,
            &b.id,
        ))
    });
    Ok(out)
}

struct TargetQuery<'a> {
    discovery: &'a RemoteDiscovery,
    name: &'a str,
    target: &'a TargetConfig,
    selection: &'a Selection,
    local_ids: &'a LocalIds,
    untracked_only: bool,
}

impl TargetQuery<'_> {
    fn collect(
        &self,
        cli_provider: &BTreeMap<String, String>,
        out: &mut Vec<RemoteEntry>,
    ) -> Result<()> {
        let app = load_installed(&self.discovery.root, &self.target.application)?;
        let auth = resolve_auth(
            &self.discovery.root,
            &self.discovery.project,
            &self.discovery.environment,
            self.target,
            cli_provider,
        )?;
        let resolved = &self.discovery.resolved[self.name];
        for (name, resource_type) in resolved.resource_types.iter().filter(|(name, _)| {
            self.selection.types.is_empty() || self.selection.types.contains(name)
        }) {
            let operation = list_operation(resource_type)?;
            let query = TypeQuery {
                target: self,
                app: &app,
                auth: &auth,
                name,
                resource_type,
                operation,
            };
            query.collect(out)?;
        }
        Ok(())
    }
}

struct TypeQuery<'a> {
    target: &'a TargetQuery<'a>,
    app: &'a ApplicationDefinition,
    auth: &'a SecretFields,
    name: &'a str,
    resource_type: &'a ResourceType,
    operation: &'a Operation,
}

impl TypeQuery<'_> {
    fn collect(&self, out: &mut Vec<RemoteEntry>) -> Result<()> {
        let discovery = self.target.discovery;
        for namespace in self.namespaces()? {
            let track = crate::hints::resolve(
                &discovery.root,
                &discovery.project,
                &discovery.environment,
                self.target.name,
                namespace,
                self.name,
            )?
            .track;
            let input = OperationInput {
                namespace,
                metadata_track: track,
                ..Default::default()
            };
            let resources =
                pagination::read(self.operation, |operation| self.read_page(operation, input))?;
            let mut seen = BTreeSet::new();
            for value in resources {
                out.extend(self.entry(namespace, value, &mut seen)?);
            }
        }
        Ok(())
    }

    fn read_page(
        &self,
        operation: &Operation,
        input: OperationInput<'_>,
    ) -> Result<Option<Vec<Value>>> {
        match execute_retry_safe(
            self.target.target,
            self.app,
            self.resource_type,
            operation,
            input,
            self.auth,
        )? {
            RemoteResult::Success(values) => Ok(Some(values)),
            RemoteResult::NotFound => Ok(None),
            RemoteResult::Conflict => bail!("List Operation reported conflict"),
            RemoteResult::Retryable | RemoteResult::Uncertain => {
                bail!("List Operation failed transiently")
            }
            RemoteResult::Failure(message) => bail!("List Operation failed: {message}"),
        }
    }

    fn namespaces(&self) -> Result<Vec<Option<&str>>> {
        let namespaces = &self.target.selection.namespaces;
        match (self.resource_type.namespaced, namespaces.is_empty()) {
            (true, true) => bail!(
                "--namespace is required to list remote Resources of namespaced type {}",
                self.name
            ),
            (false, false) => bail!(
                "--namespace is not valid for non-namespaced Resource Type {}",
                self.name
            ),
            (true, false) => Ok(namespaces
                .iter()
                .map(|value| Some(value.as_str()))
                .collect()),
            (false, true) => Ok(vec![None]),
        }
    }

    fn entry(
        &self,
        namespace: Option<&str>,
        mut value: Value,
        seen: &mut BTreeSet<String>,
    ) -> Result<Option<RemoteEntry>> {
        remove_pointer(&mut value, INTERNAL_GUARD_POINTER)?;
        let id = canonical_id(&value, self.resource_type)
            .context("remote Resource has no configured identity")?;
        let ids = &self.target.selection.ids;
        if !ids.is_empty() && !ids.contains(&id) {
            return Ok(None);
        }
        if !seen.insert(id.clone()) {
            bail!("selected List Operation returned duplicate Resource ID {id}");
        }
        let key = (
            self.target.name.to_owned(),
            namespace.map(str::to_owned),
            self.name.to_owned(),
            id.clone(),
        );
        let tracked = self.target.local_ids.contains(&key);
        if self.target.untracked_only && tracked {
            return Ok(None);
        }
        let name = display_name_value(&value, &self.resource_type.display_name)
            .unwrap_or_else(|| id.clone());
        Ok(Some(RemoteEntry {
            environment: self.target.discovery.environment.clone(),
            target: key.0,
            namespace: key.1,
            resource_type: key.2,
            id,
            name,
            tracked,
            value,
        }))
    }
}

fn list_operation(resource_type: &ResourceType) -> Result<&Operation> {
    let operation = resource_type
        .operations
        .list
        .as_ref()
        .context("selected Resource Type has no Many List Operation")?;
    if operation.cardinality != Cardinality::Many {
        bail!("List Operation must have Many cardinality");
    }
    Ok(operation)
}
