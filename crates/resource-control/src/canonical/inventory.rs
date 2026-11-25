mod input;

use super::paths::{resource_paths, validate_namespace};
use super::{InventoryEntry, Selection, resource_directories};
use crate::application::load_installed;
use crate::project::current_environment;
use crate::resolution::{ResolvedApplication, baseline_path, for_local_use, load_baseline};
use crate::{
    ApplicationDefinition, Project, ResourceType, TargetBaseline, TargetConfig, git_root,
    load_project,
};
use anyhow::{Result, bail};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub fn list_inventory(root: &Path, selection: &Selection) -> Result<Vec<InventoryEntry>> {
    list_inventory_with_resolved(root, selection, None)
}

pub(crate) fn list_inventory_with_resolved(
    root: &Path,
    selection: &Selection,
    resolved_overrides: Option<&BTreeMap<String, ResolvedApplication>>,
) -> Result<Vec<InventoryEntry>> {
    for namespace in &selection.namespaces {
        validate_namespace(namespace)?;
    }
    let root = git_root(root)?;
    let project = load_project(&root)?;
    crate::hints::validate_project_placement(&root, &project)?;
    let environment = current_environment(&root, &project, selection.environment.as_deref())?;
    let mut result = Vec::new();
    let targets = &project.environments[&environment].targets;
    for (name, target) in targets
        .iter()
        .filter(|(name, _)| selected(&selection.targets, name))
    {
        let scope = InventoryTarget {
            root: &root,
            project: &project,
            environment: &environment,
            name,
            target,
            selection,
        };
        scope.collect(
            resolved_overrides.and_then(|overrides| overrides.get(name)),
            &mut result,
        )?;
    }
    result.sort_by(|a, b| {
        (
            &a.environment,
            &a.target,
            &a.namespace,
            &a.resource_type,
            &a.id,
        )
            .cmp(&(
                &b.environment,
                &b.target,
                &b.namespace,
                &b.resource_type,
                &b.id,
            ))
    });
    check_display_names(&result)?;
    Ok(result)
}

fn selected(selection: &[String], name: &str) -> bool {
    selection.is_empty() || selection.iter().any(|candidate| candidate == name)
}

fn check_display_names(inventory: &[InventoryEntry]) -> Result<()> {
    let mut names = BTreeSet::new();
    for item in inventory.iter().filter(|item| item.display_name_unique) {
        if !names.insert((
            &item.environment,
            &item.target,
            &item.namespace,
            &item.resource_type,
            &item.display_name,
        )) {
            bail!("Display Name collision for {}", item.display_name);
        }
    }
    Ok(())
}

struct InventoryTarget<'a> {
    root: &'a Path,
    project: &'a Project,
    environment: &'a str,
    name: &'a str,
    target: &'a TargetConfig,
    selection: &'a Selection,
}

impl InventoryTarget<'_> {
    fn collect(
        &self,
        resolved_override: Option<&ResolvedApplication>,
        result: &mut Vec<InventoryEntry>,
    ) -> Result<()> {
        let app = load_installed(self.root, &self.target.application)?;
        let baseline = load_baseline(&baseline_path(self.root, self.environment, self.name)).ok();
        let fallback;
        let resolved = match resolved_override {
            Some(resolved) => resolved,
            None => {
                fallback = for_local_use(&app, self.target, baseline.as_ref())?;
                &fallback
            }
        };
        crate::hints::validate_target_tree(
            self.root,
            self.project,
            self.environment,
            self.name,
            &resolved.resource_types,
        )?;
        if let Some(baseline) = &baseline {
            self.check_unavailable_types(&app, baseline, resolved)?;
        }
        for (name, resource_type) in resolved
            .resource_types
            .iter()
            .filter(|(name, _)| selected(&self.selection.types, name))
        {
            self.collect_type(name, resource_type, result)?;
        }
        Ok(())
    }

    fn check_unavailable_types(
        &self,
        app: &ApplicationDefinition,
        baseline: &TargetBaseline,
        resolved: &ResolvedApplication,
    ) -> Result<()> {
        let version = semver::Version::parse(&baseline.application_version)?;
        let catalog = &app.catalogs[&version.major];
        for (name, definitions) in catalog
            .resource_types
            .iter()
            .filter(|(name, _)| !resolved.resource_types.contains_key(*name))
        {
            let layouts: BTreeSet<_> = definitions
                .iter()
                .map(|definition| definition.namespaced)
                .collect();
            for namespaced in layouts {
                if self.has_inputs(name, definitions, namespaced)? {
                    bail!(
                        "Resource Type {name} is unavailable for Application Version {} but has recognized inputs",
                        baseline.application_version
                    );
                }
            }
        }
        Ok(())
    }

    fn has_inputs(
        &self,
        type_name: &str,
        definitions: &[ResourceType],
        namespaced: bool,
    ) -> Result<bool> {
        for (_, directory) in self.directories(type_name, namespaced)? {
            if directory_has_recognized_input(&directory, definitions)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn directories(
        &self,
        type_name: &str,
        namespaced: bool,
    ) -> Result<Vec<(Option<String>, PathBuf)>> {
        resource_directories(
            self.root,
            self.project,
            self.environment,
            self.name,
            type_name,
            namespaced,
        )
    }

    fn collect_type(
        &self,
        name: &str,
        resource_type: &ResourceType,
        result: &mut Vec<InventoryEntry>,
    ) -> Result<()> {
        self.validate_namespace_selection(name, resource_type.namespaced)?;
        for (namespace, directory) in self.directories(name, resource_type.namespaced)? {
            if self.selection.namespaces.is_empty()
                || namespace
                    .as_ref()
                    .is_some_and(|value| self.selection.namespaces.contains(value))
            {
                let reader = input::ResourceInput {
                    scope: self,
                    name,
                    resource_type,
                    namespace: namespace.as_deref(),
                };
                reader.collect(&directory, result)?;
            }
        }
        Ok(())
    }

    fn validate_namespace_selection(&self, name: &str, namespaced: bool) -> Result<()> {
        match (
            namespaced,
            self.selection.namespaces.is_empty(),
            self.selection.ids.is_empty(),
        ) {
            (true, true, false) => bail!(
                "--namespace is required when exact IDs select namespaced Resource Type {name}"
            ),
            (false, false, _) => {
                bail!("--namespace is not valid for non-namespaced Resource Type {name}")
            }
            _ => Ok(()),
        }
    }
}

fn directory_has_recognized_input(directory: &Path, definitions: &[ResourceType]) -> Result<bool> {
    for definition in definitions {
        if !resource_paths(directory, definition)?.is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
}
