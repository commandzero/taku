use super::safe_filename;
use crate::{Project, RepositoryLayout, ResourceType};
use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub(super) fn validate_namespace(namespace: &str) -> Result<()> {
    if !is_path_segment(namespace) {
        bail!("Namespace must be one non-empty path segment");
    }
    Ok(())
}

fn is_path_segment(name: &str) -> bool {
    !name.is_empty() && !matches!(name, "." | "..") && !name.contains(['/', '\\'])
}

pub(crate) fn target_root(
    root: &Path,
    project: &Project,
    environment: &str,
    target: &str,
) -> PathBuf {
    match project.layout {
        RepositoryLayout::Single => root.join(target),
        RepositoryLayout::Multi => root.join(environment).join(target),
    }
}

pub(super) fn resource_directory_in_namespace(
    target_root: PathBuf,
    namespace: Option<&str>,
    resource_type: &str,
) -> PathBuf {
    match namespace {
        Some(namespace) => target_root.join(namespace).join(resource_type),
        None => target_root.join(resource_type),
    }
}

pub(crate) fn resource_path(
    directory: &Path,
    value: &Value,
    resource_type: &ResourceType,
    filename: &str,
) -> Result<PathBuf> {
    if !is_path_segment(filename) {
        bail!("Resource filename is not one safe path segment");
    }
    let directory = if let Some(pointer) = &resource_type.directory_pointer {
        let value = value
            .pointer(pointer)
            .and_then(Value::as_str)
            .with_context(|| {
                format!("Resource directory pointer {pointer} is missing or is not a string")
            })?;
        let directory_name = safe_filename(value);
        if directory_name.is_empty() {
            bail!("Resource directory pointer {pointer} resolves to an empty path segment");
        }
        directory.join(directory_name)
    } else {
        directory.to_owned()
    };
    Ok(directory.join(filename))
}

pub(crate) fn resource_paths(
    directory: &Path,
    resource_type: &ResourceType,
) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    collect_resource_paths(directory, resource_type, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn collect_resource_paths(
    directory: &Path,
    resource_type: &ResourceType,
    paths: &mut Vec<PathBuf>,
) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            bail!(
                "symlinked Resource input is not allowed: {}",
                path.display()
            );
        }
        if path.file_name().and_then(|name| name.to_str()) == Some(crate::hints::RESOURCE_HINT_NAME)
        {
            continue;
        }
        if accepts_resource_path(resource_type, &path, &metadata) {
            paths.push(path);
        } else if should_walk_directory(resource_type, &metadata) {
            collect_resource_paths(&path, resource_type, paths)?;
        }
    }
    Ok(())
}

pub(super) fn resource_directories(
    target_root: PathBuf,
    resource_type: &str,
    namespaced: bool,
) -> Result<Vec<(Option<String>, PathBuf)>> {
    if !namespaced {
        let directory = target_root.join(resource_type);
        return Ok(directory
            .is_dir()
            .then_some((None, directory))
            .into_iter()
            .collect());
    }

    if !target_root.is_dir() {
        return Ok(Vec::new());
    }
    let mut directories = Vec::new();
    for entry in fs::read_dir(target_root)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if !metadata.is_dir() {
            continue;
        }
        let namespace = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Namespace directory is not valid UTF-8"))?;
        let directory = entry.path().join(resource_type);
        if directory.is_dir() {
            directories.push((Some(namespace), directory));
        }
    }
    directories.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(directories)
}

fn accepts_resource_path(
    resource_type: &ResourceType,
    path: &Path,
    metadata: &fs::Metadata,
) -> bool {
    if resource_type.filesystem.is_some() {
        return metadata.is_dir();
    }
    metadata.is_file() && (is_deletion_marker(path) || has_resource_extension(path))
}

fn should_walk_directory(resource_type: &ResourceType, metadata: &fs::Metadata) -> bool {
    metadata.is_dir()
        && resource_type.filesystem.is_none()
        && resource_type.directory_pointer.is_some()
}

pub(super) fn has_resource_extension(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("json" | "json5" | "yaml" | "yml")
    )
}

pub(crate) fn is_deletion_marker(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with(".delete.yaml"))
}

pub(crate) fn reject_symlink_components(root: &Path, path: &Path) -> Result<()> {
    let relative = path
        .strip_prefix(root)
        .context("Resource path escapes Project")?;
    let mut current = root.to_owned();
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            bail!("invalid Resource path");
        }
        current.push(component);
        if fs::symlink_metadata(&current).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!(
                "symlinked Resource directory is not allowed: {}",
                current.display()
            );
        }
    }
    Ok(())
}
