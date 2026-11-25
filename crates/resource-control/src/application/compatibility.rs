use super::load_installed;
use crate::canonical::{
    canonical_id, is_deletion_marker, parse_resource, resource_directories, resource_paths,
};
use crate::resolution::{baseline_path, for_local_use, load_baseline};
use crate::{ApplicationDefinition, Project, ResourceType, TargetConfig, load_project};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::fs;
use std::path::Path;

pub(super) fn validate_resources_for(
    root: &Path,
    name: &str,
    definition: &ApplicationDefinition,
) -> Result<()> {
    let project = load_project(root)?;
    let Ok(installed) = load_installed(root, name) else {
        return Ok(());
    };
    let update = CompatibilityCheck {
        root,
        project: &project,
        installed: &installed,
        definition,
    };
    for (environment, env) in &project.environments {
        for (target_name, target) in env
            .targets
            .iter()
            .filter(|(_, target)| target.application == name)
        {
            update.check_target(environment, target_name, target)?;
        }
    }
    Ok(())
}

struct CompatibilityCheck<'a> {
    root: &'a Path,
    project: &'a Project,
    installed: &'a ApplicationDefinition,
    definition: &'a ApplicationDefinition,
}

impl CompatibilityCheck<'_> {
    fn check_target(
        &self,
        environment: &str,
        target_name: &str,
        target: &TargetConfig,
    ) -> Result<()> {
        let baseline = load_baseline(&baseline_path(self.root, environment, target_name)).ok();
        let installed = for_local_use(self.installed, target, baseline.as_ref())?;
        let updated = for_local_use(self.definition, target, baseline.as_ref())?;
        for (name, installed_type) in &installed.resource_types {
            let directories = resource_directories(
                self.root,
                self.project,
                environment,
                target_name,
                name,
                installed_type.namespaced,
            )?;
            if directories.is_empty() {
                continue;
            }
            let updated_type = updated.resource_types.get(name).with_context(|| {
                format!("updated Application removes managed Resource Type {name}")
            })?;
            check_layout(name, installed_type, updated_type)?;
            for (_, directory) in directories {
                check_directory(&directory, updated_type)?;
            }
        }
        Ok(())
    }
}

fn check_layout(name: &str, installed: &ResourceType, updated: &ResourceType) -> Result<()> {
    if updated.namespaced != installed.namespaced {
        bail!("updated Application changes namespace layout for managed Resource Type {name}");
    }
    if serde_yaml::to_string(&updated.filesystem)? != serde_yaml::to_string(&installed.filesystem)?
    {
        bail!("updated Application changes filesystem projection for managed Resource Type {name}");
    }
    Ok(())
}

fn check_directory(directory: &Path, resource_type: &ResourceType) -> Result<()> {
    for path in resource_paths(directory, resource_type)? {
        let Some(value) = read_value(&path, resource_type)? else {
            continue;
        };
        if canonical_id(&value, resource_type).is_none() {
            bail!(
                "Resource {} is incompatible with updated Application",
                path.display()
            );
        }
    }
    Ok(())
}

fn read_value(path: &Path, resource_type: &ResourceType) -> Result<Option<Value>> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        bail!(
            "symlinked Resource input is not allowed: {}",
            path.display()
        );
    }
    match &resource_type.filesystem {
        Some(projection) if metadata.is_dir() => {
            crate::projection::merge(path, projection).map(Some)
        }
        None if metadata.is_file() && !is_deletion_marker(path) => parse_resource(path).map(Some),
        _ => Ok(None),
    }
}
