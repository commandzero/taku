use crate::application::load_installed;
use crate::canonical::{
    Selection, is_deletion_marker, reject_symlink_components, resource_directories, resource_paths,
};
use crate::project::current_environment;
use crate::resolution::{baseline_path, for_local_use, load_baseline};
use crate::{Project, ResourceType, SCHEMA_VERSION, TargetConfig, git_root, load_project};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeletionMarker {
    pub schema_version: u32,
    pub environment: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parameters: BTreeMap<String, String>,
    pub guard: String,
    pub source_path: String,
}

fn load_deletion_marker(scope: &MarkerDirectory<'_>, path: &Path) -> Result<DeletionMarker> {
    let root = scope.target.root;
    reject_symlink_components(root, path)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        bail!(
            "symlinked Deletion Marker input is not allowed: {}",
            path.display()
        );
    }
    if !metadata.is_file() {
        bail!("Deletion Marker is not a regular file: {}", path.display());
    }
    let marker: DeletionMarker = serde_yaml::from_str(&fs::read_to_string(path)?)?;
    if marker.schema_version != SCHEMA_VERSION {
        bail!(
            "unsupported Deletion Marker schema version {}",
            marker.schema_version
        );
    }
    if !scope.matches_binding(&marker) {
        bail!("Deletion Marker binding does not match its Environment/Target/Resource Type tree");
    }
    let source = Path::new(&marker.source_path);
    if source.is_absolute() {
        bail!("Deletion Marker source_path must be Project-relative");
    }
    let expected = root.join(source).with_extension("delete.yaml");
    reject_symlink_components(root, &root.join(source))?;
    if expected != path {
        bail!("Deletion Marker source_path does not match its marker path");
    }
    Ok(marker)
}

pub fn deletion_markers(
    root: &Path,
    selection: &Selection,
) -> Result<Vec<(PathBuf, DeletionMarker)>> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let environment = current_environment(&root, &project, selection.environment.as_deref())?;
    let mut out = Vec::new();
    let scan = MarkerScan {
        root: &root,
        project: &project,
        environment: &environment,
        selection,
    };
    scan.visit(|path, marker| {
        out.push((path, marker));
        Ok(())
    })?;
    Ok(out)
}

pub(super) struct MarkerScan<'a> {
    pub(super) root: &'a Path,
    pub(super) project: &'a Project,
    pub(super) environment: &'a str,
    pub(super) selection: &'a Selection,
}

impl MarkerScan<'_> {
    pub(super) fn visit(
        &self,
        mut visit: impl FnMut(PathBuf, DeletionMarker) -> Result<()>,
    ) -> Result<()> {
        let targets = &self.project.environments[self.environment].targets;
        for (name, target) in targets
            .iter()
            .filter(|(name, _)| selected(&self.selection.targets, name))
        {
            self.visit_target(name, target, &mut visit)?;
        }
        Ok(())
    }

    fn visit_target(
        &self,
        name: &str,
        target: &TargetConfig,
        visit: &mut impl FnMut(PathBuf, DeletionMarker) -> Result<()>,
    ) -> Result<()> {
        let app = load_installed(self.root, &target.application)?;
        let baseline = load_baseline(&baseline_path(self.root, self.environment, name)).ok();
        let resolved = for_local_use(&app, target, baseline.as_ref())?;
        let scan = MarkerTarget {
            root: self.root,
            project: self.project,
            environment: self.environment,
            name,
            selection: self.selection,
        };
        for (name, resource_type) in resolved
            .resource_types
            .iter()
            .filter(|(name, _)| selected(&self.selection.types, name))
        {
            scan.visit_type(name, resource_type, visit)?;
        }
        Ok(())
    }
}

fn selected(values: &[String], value: &str) -> bool {
    values.is_empty() || values.iter().any(|candidate| candidate == value)
}

struct MarkerTarget<'a> {
    root: &'a Path,
    project: &'a Project,
    environment: &'a str,
    name: &'a str,
    selection: &'a Selection,
}

impl MarkerTarget<'_> {
    fn visit_type(
        &self,
        name: &str,
        resource_type: &ResourceType,
        visit: &mut impl FnMut(PathBuf, DeletionMarker) -> Result<()>,
    ) -> Result<()> {
        for (namespace, directory) in resource_directories(
            self.root,
            self.project,
            self.environment,
            self.name,
            name,
            resource_type.namespaced,
        )? {
            let namespaces = &self.selection.namespaces;
            if namespaces.is_empty()
                || namespace
                    .as_ref()
                    .is_some_and(|value| namespaces.contains(value))
            {
                let scope = MarkerDirectory {
                    target: self,
                    resource_type: name,
                    namespace: namespace.as_deref(),
                };
                scope.visit(&directory, resource_type, visit)?;
            }
        }
        Ok(())
    }
}

struct MarkerDirectory<'a> {
    target: &'a MarkerTarget<'a>,
    resource_type: &'a str,
    namespace: Option<&'a str>,
}

impl MarkerDirectory<'_> {
    fn visit(
        &self,
        directory: &Path,
        resource_type: &ResourceType,
        visit: &mut impl FnMut(PathBuf, DeletionMarker) -> Result<()>,
    ) -> Result<()> {
        reject_symlink_components(self.target.root, directory)?;
        for path in resource_paths(directory, resource_type)?
            .into_iter()
            .filter(|path| is_deletion_marker(path))
        {
            let marker = load_deletion_marker(self, &path)?;
            if selected(&self.target.selection.ids, &marker.id) {
                visit(path, marker)?;
            }
        }
        Ok(())
    }

    fn matches_binding(&self, marker: &DeletionMarker) -> bool {
        (
            marker.environment.as_str(),
            marker.target.as_str(),
            marker.namespace.as_deref(),
            marker.resource_type.as_str(),
        ) == (
            self.target.environment,
            self.target.name,
            self.namespace,
            self.resource_type,
        )
    }
}
