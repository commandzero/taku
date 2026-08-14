use crate::application::load_installed;
use crate::canonical::{
    Selection, list_inventory, reject_symlink_components, resource_directory, safe_filename,
    write_resource,
};
use crate::observe::{cache_path, load_observation, remote_list};
use crate::project::current_environment;
use crate::{IdScope, RepositoryLayout, SCHEMA_VERSION, git_root, load_project};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Serialize)]
pub struct LifecycleResult {
    pub environment: String,
    pub target: String,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
    pub outcome: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeletionMarker {
    pub schema_version: u32,
    pub environment: String,
    pub target: String,
    #[serde(rename = "type")]
    pub resource_type: String,
    pub id: String,
    pub guard: String,
    pub source_path: String,
}

pub fn add_remote(
    root: &Path,
    selection: &Selection,
    provider: &BTreeMap<String, String>,
) -> Result<Vec<LifecycleResult>> {
    if selection.ids.is_empty() {
        bail!("Add requires at least one exact --id selector");
    }
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let remote = remote_list(&root, selection, false, provider)?;
    let mut out = Vec::new();
    for item in remote {
        if item.tracked {
            continue;
        }
        let app = load_installed(
            &root,
            &project.environments[&item.environment].targets[&item.target].application,
        )?;
        let rt = &app.target_profile.resource_types[&item.resource_type];
        let name = crate::canonical::pointer_string(&item.value, &rt.display_name.pointer)
            .unwrap_or_else(|| item.id.clone());
        let display = match rt.display_name.strategy {
            crate::DisplayNameStrategy::Id => item.id.clone(),
            crate::DisplayNameStrategy::Name => name,
            crate::DisplayNameStrategy::NameId => {
                format!("{}-{}", name, crate::canonical::short_id(&item.id))
            }
        };
        let path = resource_directory(
            &root,
            &project,
            &item.environment,
            &item.target,
            &item.resource_type,
        )
        .join(format!("{}.json", safe_filename(&display)));
        if path.exists() {
            bail!("Resource destination already exists: {}", path.display());
        }
        write_resource(&path, &item.value)?;
        out.push(LifecycleResult {
            environment: item.environment,
            target: item.target,
            resource_type: item.resource_type,
            id: item.id,
            outcome: "added".into(),
        });
    }
    for id in &selection.ids {
        if !out.iter().any(|r| &r.id == id) {
            bail!("selected remote Resource {id} was not found or is already managed");
        }
    }
    Ok(out)
}

pub fn remove(root: &Path, selection: &Selection) -> Result<Vec<LifecycleResult>> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let environment = current_environment(&root, &project, selection.environment.as_deref())?;
    let inventory = list_inventory(&root, selection)?;
    let mut out = Vec::new();
    for item in inventory {
        let observation = load_observation(&cache_path(
            &root,
            &environment,
            &item.target,
            &item.resource_type,
        ))?;
        let observed = observation
            .resources
            .get(&item.id)
            .context("Deletion Marker requires Observed State; run `taku fetch`")?;
        let guard = observed
            .guard
            .clone()
            .context("Deletion Marker requires a present observed identity guard")?;
        let source = root.join(&item.path);
        let marker_path = source.with_extension("delete.yml");
        let marker = DeletionMarker {
            schema_version: SCHEMA_VERSION,
            environment: environment.clone(),
            target: item.target.clone(),
            resource_type: item.resource_type.clone(),
            id: item.id.clone(),
            guard,
            source_path: item.path.clone(),
        };
        fs::write(&marker_path, serde_yaml::to_string(&marker)?)?;
        fs::remove_file(&source)?;
        out.push(LifecycleResult {
            environment: environment.clone(),
            target: item.target,
            resource_type: item.resource_type,
            id: item.id,
            outcome: "marked_for_deletion".into(),
        });
    }
    Ok(out)
}

pub fn forget(root: &Path, selection: &Selection) -> Result<Vec<LifecycleResult>> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let environment = current_environment(&root, &project, selection.environment.as_deref())?;
    let mut out = Vec::new();
    for item in list_inventory(&root, selection)? {
        fs::remove_file(root.join(&item.path))?;
        out.push(LifecycleResult {
            environment: environment.clone(),
            target: item.target,
            resource_type: item.resource_type,
            id: item.id,
            outcome: "forgotten".into(),
        });
    }
    for (target_name, target) in &project.environments[&environment].targets {
        if !selection.targets.is_empty() && !selection.targets.contains(target_name) {
            continue;
        }
        let app = load_installed(&root, &target.application)?;
        for type_name in app.target_profile.resource_types.keys() {
            if !selection.types.is_empty() && !selection.types.contains(type_name) {
                continue;
            }
            let dir = resource_directory(&root, &project, &environment, target_name, type_name);
            if dir.exists() {
                reject_symlink_components(&root, &dir)?;
            }
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries {
                    let path = entry?.path();
                    if !path
                        .file_name()
                        .and_then(|v| v.to_str())
                        .is_some_and(|v| v.ends_with(".delete.yml"))
                    {
                        continue;
                    }
                    let marker =
                        load_deletion_marker(&root, &path, &environment, target_name, type_name)?;
                    if !selection.ids.is_empty() && !selection.ids.contains(&marker.id) {
                        continue;
                    }
                    fs::remove_file(path)?;
                    out.push(LifecycleResult {
                        environment: environment.clone(),
                        target: target_name.clone(),
                        resource_type: type_name.clone(),
                        id: marker.id,
                        outcome: "forgotten".into(),
                    });
                }
            }
        }
    }
    Ok(out)
}

pub fn deletion_markers(
    root: &Path,
    selection: &Selection,
) -> Result<Vec<(std::path::PathBuf, DeletionMarker)>> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let environment = current_environment(&root, &project, selection.environment.as_deref())?;
    let mut out = Vec::new();
    for (target_name, target) in &project.environments[&environment].targets {
        if !selection.targets.is_empty() && !selection.targets.contains(target_name) {
            continue;
        }
        let app = load_installed(&root, &target.application)?;
        for type_name in app.target_profile.resource_types.keys() {
            if !selection.types.is_empty() && !selection.types.contains(type_name) {
                continue;
            }
            let dir = resource_directory(&root, &project, &environment, target_name, type_name);
            if dir.exists() {
                reject_symlink_components(&root, &dir)?;
            }
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries {
                    let path = entry?.path();
                    if path
                        .file_name()
                        .and_then(|v| v.to_str())
                        .is_some_and(|v| v.ends_with(".delete.yml"))
                    {
                        let marker = load_deletion_marker(
                            &root,
                            &path,
                            &environment,
                            target_name,
                            type_name,
                        )?;
                        if selection.ids.is_empty() || selection.ids.contains(&marker.id) {
                            out.push((path, marker));
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

fn load_deletion_marker(
    root: &Path,
    path: &Path,
    environment: &str,
    target: &str,
    resource_type: &str,
) -> Result<DeletionMarker> {
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
    if marker.environment != environment
        || marker.target != target
        || marker.resource_type != resource_type
    {
        bail!("Deletion Marker binding does not match its Environment/Target/Resource Type tree");
    }
    let source = Path::new(&marker.source_path);
    if source.is_absolute() {
        bail!("Deletion Marker source_path must be Project-relative");
    }
    let expected = root.join(source).with_extension("delete.yml");
    reject_symlink_components(root, &root.join(source))?;
    if expected != path {
        bail!("Deletion Marker source_path does not match its marker path");
    }
    Ok(marker)
}

#[derive(Clone, Debug, Serialize)]
pub struct PromotionResult {
    pub from_environment: String,
    pub to_environment: String,
    pub from_target: String,
    pub to_target: String,
    pub id: String,
    pub outcome: String,
}
pub fn promote(
    root: &Path,
    from_environment: Option<&str>,
    to_environment: Option<&str>,
    from_target: Option<&str>,
    to_target: Option<&str>,
) -> Result<Vec<PromotionResult>> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    if project.layout != RepositoryLayout::Multi {
        bail!("Single-layout Promotion requires explicit source and destination Projects");
    }
    let destination = current_environment(&root, &project, to_environment)?;
    let source = from_environment
        .map(str::to_owned)
        .or_else(|| project.environments[&destination].from.clone())
        .context("destination Environment has no upstream `from` mapping")?;
    let mut pairs = Vec::new();
    if let Some(to) = to_target {
        let target = &project.environments[&destination].targets[to];
        pairs.push((
            target
                .from
                .clone()
                .or_else(|| from_target.map(str::to_owned))
                .unwrap_or_else(|| to.into()),
            to.to_owned(),
        ));
    } else {
        for (to, target) in &project.environments[&destination].targets {
            if let Some(filter) = from_target
                && target.from.as_deref().unwrap_or(to) != filter
            {
                continue;
            }
            pairs.push((
                target.from.clone().unwrap_or_else(|| to.clone()),
                to.clone(),
            ));
        }
    }
    let mut out = Vec::new();
    for (from, to) in pairs {
        let Some(source_target) = project.environments[&source].targets.get(&from) else {
            out.push(PromotionResult {
                from_environment: source.clone(),
                to_environment: destination.clone(),
                from_target: from,
                to_target: to,
                id: "".into(),
                outcome: "skipped_unresolved".into(),
            });
            continue;
        };
        let destination_target = &project.environments[&destination].targets[&to];
        if source_target.application != destination_target.application {
            bail!("Promotion Targets {from} and {to} use incompatible Applications");
        }
        let selection = Selection {
            environment: Some(source.clone()),
            targets: vec![from.clone()],
            types: vec![],
            ids: vec![],
        };
        for item in list_inventory(&root, &selection)? {
            if item.id_scope != IdScope::Universal {
                bail!(
                    "Resource {} has Target-scoped identity and cannot be promoted",
                    item.id
                );
            }
            let source_path = Path::new(&item.path);
            let filename = source_path.file_name().unwrap();
            let destination_path =
                resource_directory(&root, &project, &destination, &to, &item.resource_type)
                    .join(filename);
            write_resource(&destination_path, &item.value)?;
            out.push(PromotionResult {
                from_environment: source.clone(),
                to_environment: destination.clone(),
                from_target: from.clone(),
                to_target: to.clone(),
                id: item.id,
                outcome: "promoted".into(),
            });
        }
    }
    Ok(out)
}

pub fn promote_projects(
    source_root: &Path,
    destination_root: &Path,
    from_target: &str,
    to_target: &str,
) -> Result<Vec<PromotionResult>> {
    let source_root = git_root(source_root)?;
    let destination_root = git_root(destination_root)?;
    let source_project = load_project(&source_root)?;
    let destination_project = load_project(&destination_root)?;
    if source_project.layout != RepositoryLayout::Single
        || destination_project.layout != RepositoryLayout::Single
    {
        bail!("explicit cross-Project Promotion requires two Single-layout Projects");
    }
    let source_environment = source_project.environments.keys().next().unwrap().clone();
    let destination_environment = destination_project
        .environments
        .keys()
        .next()
        .unwrap()
        .clone();
    let source_target = &source_project.environments[&source_environment].targets[from_target];
    let destination_target =
        &destination_project.environments[&destination_environment].targets[to_target];
    if source_target.application != destination_target.application {
        bail!("Promotion Targets use incompatible Applications");
    }
    let source_app = load_installed(&source_root, &source_target.application)?;
    let destination_app = load_installed(&destination_root, &destination_target.application)?;
    if serde_yaml::to_string(&source_app)? != serde_yaml::to_string(&destination_app)? {
        bail!("Promotion installed Applications are not exactly compatible");
    }
    let selection = Selection {
        environment: None,
        targets: vec![from_target.into()],
        types: vec![],
        ids: vec![],
    };
    let mut out = Vec::new();
    for item in list_inventory(&source_root, &selection)? {
        if item.id_scope != IdScope::Universal {
            bail!(
                "Resource {} has Target-scoped identity and cannot be promoted",
                item.id
            );
        }
        let filename = Path::new(&item.path).file_name().unwrap();
        let path = resource_directory(
            &destination_root,
            &destination_project,
            &destination_environment,
            to_target,
            &item.resource_type,
        )
        .join(filename);
        write_resource(&path, &item.value)?;
        out.push(PromotionResult {
            from_environment: source_environment.clone(),
            to_environment: destination_environment.clone(),
            from_target: from_target.into(),
            to_target: to_target.into(),
            id: item.id,
            outcome: "promoted".into(),
        });
    }
    Ok(out)
}
