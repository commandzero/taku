use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const TARGET_HINT_NAME: &str = ".target.yaml";
pub(crate) const RESOURCE_HINT_NAME: &str = ".resource.yaml";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct TargetHints {
    pub schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<MetadataTrackingHint>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResourceHints {
    pub schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<MetadataTrackingHint>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct MetadataTrackingHint {
    pub track: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct HintResolution {
    pub track: bool,
    pub target_path: PathBuf,
    pub resource_path: PathBuf,
    pub target_bytes: Option<Vec<u8>>,
    pub resource_bytes: Option<Vec<u8>>,
}

impl HintResolution {
    pub(crate) fn binding_material(&self, root: &Path) -> Result<Vec<u8>> {
        let mut material = Vec::new();
        for (path, bytes) in [
            (&self.target_path, &self.target_bytes),
            (&self.resource_path, &self.resource_bytes),
        ] {
            let relative = path
                .strip_prefix(root)
                .context("hint path escapes Project")?;
            let encoded = relative.to_string_lossy();
            material.extend_from_slice(&(encoded.len() as u64).to_be_bytes());
            material.extend_from_slice(encoded.as_bytes());
            match bytes {
                Some(bytes) => {
                    material.push(1);
                    material.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
                    material.extend_from_slice(bytes);
                }
                None => material.push(0),
            }
        }
        Ok(material)
    }
}

fn validate_version(version: u32, kind: &str) -> Result<()> {
    if version != crate::SCHEMA_VERSION {
        bail!("unsupported {kind} hint schema version {version}");
    }
    Ok(())
}

pub(crate) fn parse_target_hints(bytes: &[u8]) -> Result<TargetHints> {
    let hints: TargetHints = serde_yaml::from_slice(bytes).context("invalid .target.yaml")?;
    validate_version(hints.schema_version, "Target")?;
    Ok(hints)
}

pub(crate) fn parse_resource_hints(bytes: &[u8]) -> Result<ResourceHints> {
    let hints: ResourceHints = serde_yaml::from_slice(bytes).context("invalid .resource.yaml")?;
    validate_version(hints.schema_version, "Resource Type")?;
    Ok(hints)
}

fn read_hint(root: &Path, path: &Path, kind: &str) -> Result<Option<(Vec<u8>, Option<bool>)>> {
    crate::canonical::reject_symlink_components(root, path)?;
    if !path.exists() {
        return Ok(None);
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("{kind} hint is not a regular file: {}", path.display());
    }
    let bytes = fs::read(path)?;
    let track = match kind {
        "Target" => parse_target_hints(&bytes)?
            .metadata
            .map(|value| value.track),
        _ => parse_resource_hints(&bytes)?
            .metadata
            .map(|value| value.track),
    };
    Ok(Some((bytes, track)))
}

pub(crate) fn resolve(
    root: &Path,
    project: &crate::Project,
    environment: &str,
    target: &str,
    namespace: Option<&str>,
    resource_type: &str,
) -> Result<HintResolution> {
    let target_path =
        crate::canonical::target_root(root, project, environment, target).join(TARGET_HINT_NAME);
    let resource_path = crate::canonical::resource_directory_in_namespace(
        root,
        project,
        environment,
        target,
        namespace,
        resource_type,
    )
    .join(RESOURCE_HINT_NAME);
    let target_hint = read_hint(root, &target_path, "Target")?;
    let resource_hint = read_hint(root, &resource_path, "Resource Type")?;
    let track = resource_hint
        .as_ref()
        .and_then(|(_, track)| *track)
        .or_else(|| target_hint.as_ref().and_then(|(_, track)| *track))
        .unwrap_or(false);
    Ok(HintResolution {
        track,
        target_path,
        resource_path,
        target_bytes: target_hint.map(|(bytes, _)| bytes),
        resource_bytes: resource_hint.map(|(bytes, _)| bytes),
    })
}

pub(crate) fn validate_target_tree(
    root: &Path,
    project: &crate::Project,
    environment: &str,
    target: &str,
    resource_types: &std::collections::BTreeMap<String, crate::ResourceType>,
) -> Result<()> {
    let target_root = crate::canonical::target_root(root, project, environment, target);
    if !target_root.exists() {
        return Ok(());
    }
    crate::canonical::reject_symlink_components(root, &target_root)?;
    validate_tree_entries(root, &target_root, &target_root, resource_types)
}

pub(crate) fn validate_application_tree(
    root: &Path,
    project: &crate::Project,
    environment: &str,
    target: &str,
    application: &crate::ApplicationDefinition,
) -> Result<()> {
    let mut candidates = std::collections::BTreeMap::new();
    for catalog in application.catalogs.values() {
        for (name, definitions) in &catalog.resource_types {
            for definition in definitions {
                candidates
                    .entry(name.clone())
                    .or_insert_with(|| definition.clone());
            }
        }
    }
    validate_target_tree(root, project, environment, target, &candidates)
}

pub(crate) fn validate_project_placement(root: &Path, project: &crate::Project) -> Result<()> {
    let target_roots = project
        .environments
        .iter()
        .flat_map(|(environment, config)| {
            config.targets.keys().map(move |target| {
                crate::canonical::target_root(root, project, environment, target)
            })
        })
        .collect::<Vec<_>>();
    let mut reserved = Vec::new();
    find_reserved(root, root, &mut reserved)?;
    for path in reserved {
        if !target_roots
            .iter()
            .any(|target_root| path.starts_with(target_root))
        {
            bail!(
                "reserved hint belongs to an unknown Environment or Target: {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn find_reserved(root: &Path, directory: &Path, found: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if directory == root && matches!(name.as_ref(), ".git" | ".taku") {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        if name == TARGET_HINT_NAME || name == RESOURCE_HINT_NAME {
            found.push(path);
        } else if metadata.is_dir() && !metadata.file_type().is_symlink() {
            find_reserved(root, &path, found)?;
        }
    }
    Ok(())
}

pub(crate) fn validate_tracking(
    resource_type: &crate::ResourceType,
    hints: &HintResolution,
    type_name: &str,
) -> Result<()> {
    if !hints.track {
        return Ok(());
    }
    let Some(metadata) = &resource_type.metadata else {
        return Ok(());
    };
    for pointer in &metadata.fields {
        if resource_type
            .sensitive_fields
            .iter()
            .any(|sensitive| pointers_overlap(pointer, sensitive))
        {
            bail!(
                "tracked metadata field {pointer} overlaps a Target Sensitive Field for Resource Type {type_name}"
            );
        }
    }
    Ok(())
}

fn pointers_overlap(left: &str, right: &str) -> bool {
    left == right
        || left.starts_with(&format!("{right}/"))
        || right.starts_with(&format!("{left}/"))
}

fn validate_tree_entries(
    root: &Path,
    target_root: &Path,
    directory: &Path,
    resource_types: &std::collections::BTreeMap<String, crate::ResourceType>,
) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            bail!("symlinked managed input is not allowed: {}", path.display());
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == TARGET_HINT_NAME || name == RESOURCE_HINT_NAME {
            if !metadata.is_file() {
                bail!("hint is not a regular file: {}", path.display());
            }
            let relative = path.strip_prefix(target_root)?;
            let components = relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            let valid = if name == TARGET_HINT_NAME {
                components == [TARGET_HINT_NAME]
            } else {
                match components.as_slice() {
                    [type_name, hint]
                        if hint == RESOURCE_HINT_NAME
                            && resource_types
                                .get(type_name)
                                .is_some_and(|resource_type| !resource_type.namespaced) =>
                    {
                        true
                    }
                    [namespace, type_name, hint]
                        if hint == RESOURCE_HINT_NAME
                            && !namespace.is_empty()
                            && namespace != "."
                            && namespace != ".."
                            && resource_types
                                .get(type_name)
                                .is_some_and(|resource_type| resource_type.namespaced) =>
                    {
                        true
                    }
                    _ => false,
                }
            };
            if !valid {
                bail!("reserved hint has invalid placement: {}", path.display());
            }
            read_hint(
                root,
                &path,
                if name == TARGET_HINT_NAME {
                    "Target"
                } else {
                    "Resource Type"
                },
            )?;
        } else if metadata.is_dir() {
            validate_tree_entries(root, target_root, &path, resource_types)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        parse_resource_hints, parse_target_hints, resolve, validate_project_placement,
        validate_target_tree,
    };
    use crate::{EnvironmentConfig, Project, RepositoryLayout};
    use std::collections::BTreeMap;

    #[test]
    fn closed_versioned_hints_parse_true_false_and_round_trip() {
        let target = parse_target_hints(b"schema_version: 1\nmetadata:\n  track: true\n").unwrap();
        assert!(target.metadata.unwrap().track);

        let resource =
            parse_resource_hints(b"schema_version: 1\nmetadata:\n  track: false\n").unwrap();
        assert!(!resource.metadata.as_ref().unwrap().track);
        let yaml = serde_yaml::to_string(&resource).unwrap();
        assert_eq!(parse_resource_hints(yaml.as_bytes()).unwrap(), resource);
    }

    #[test]
    fn closed_versioned_hints_reject_invalid_input() {
        for yaml in [
            "schema_version: 2\nmetadata:\n  track: true\n",
            "schema_version: 1\nmetadata:\n  track: sometimes\n",
            "schema_version: 1\nmetadata:\n  track: true\n  extra: false\n",
            "schema_version: 1\napplication: example\n",
        ] {
            assert!(parse_target_hints(yaml.as_bytes()).is_err(), "{yaml}");
            assert!(parse_resource_hints(yaml.as_bytes()).is_err(), "{yaml}");
        }
    }

    fn project(layout: RepositoryLayout) -> Project {
        Project {
            schema_version: 1,
            layout,
            environments: BTreeMap::from([("dev".into(), EnvironmentConfig::default())]),
            application_source: None,
            push: Default::default(),
            max_requests: 4,
        }
    }

    #[test]
    fn closest_physical_directory_hint_wins() {
        let root = tempfile::tempdir().unwrap();
        let project = project(RepositoryLayout::Single);
        std::fs::create_dir_all(root.path().join("api/space-a/widgets")).unwrap();
        std::fs::create_dir_all(root.path().join("api/space-b/widgets")).unwrap();
        std::fs::write(
            root.path().join("api/.target.yaml"),
            "schema_version: 1\nmetadata: { track: true }\n",
        )
        .unwrap();
        std::fs::write(
            root.path().join("api/space-a/widgets/.resource.yaml"),
            "schema_version: 1\nmetadata: { track: false }\n",
        )
        .unwrap();

        assert!(
            !resolve(
                root.path(),
                &project,
                "dev",
                "api",
                Some("space-a"),
                "widgets"
            )
            .unwrap()
            .track
        );
        assert!(
            resolve(
                root.path(),
                &project,
                "dev",
                "api",
                Some("space-b"),
                "widgets"
            )
            .unwrap()
            .track
        );
        assert!(
            !resolve(root.path(), &project, "dev", "other", None, "widgets")
                .unwrap()
                .track
        );
    }

    #[test]
    fn multi_layout_uses_environment_target_root() {
        let root = tempfile::tempdir().unwrap();
        let project = project(RepositoryLayout::Multi);
        std::fs::create_dir_all(root.path().join("dev/api/widgets")).unwrap();
        std::fs::write(
            root.path().join("dev/api/.target.yaml"),
            "schema_version: 1\nmetadata: { track: true }\n",
        )
        .unwrap();

        let resolved = resolve(root.path(), &project, "dev", "api", None, "widgets").unwrap();
        assert!(resolved.track);
        assert_eq!(
            resolved.target_path,
            root.path().join("dev/api/.target.yaml")
        );
        assert_eq!(
            resolved.resource_path,
            root.path().join("dev/api/widgets/.resource.yaml")
        );
    }

    #[test]
    fn reserved_hints_are_rejected_outside_their_exact_scope() {
        let root = tempfile::tempdir().unwrap();
        let project = project(RepositoryLayout::Single);
        let resource_type: crate::ResourceType = serde_yaml::from_str(
            r#"
id: { pointer: /id, scope: universal }
display_name: { strategy: id }
filesystem:
  split: frontmatter_markdown
  merge: frontmatter_markdown
  frontmatter_markdown:
    document: document.md
    body_pointer: /body
    referenced_files:
      pointer: /files
      path_pointer: /path
      name_pointer: /name
      content_pointer: /content
      extension: txt
operations: {}
"#,
        )
        .unwrap();
        let definitions = BTreeMap::from([("widgets".into(), resource_type)]);
        std::fs::create_dir_all(root.path().join("api/widgets/one")).unwrap();
        std::fs::write(
            root.path().join("api/widgets/one/.resource.yaml"),
            "schema_version: 1\nmetadata: { track: true }\n",
        )
        .unwrap();

        assert!(validate_target_tree(root.path(), &project, "dev", "api", &definitions).is_err());
    }

    #[test]
    fn hint_for_an_unavailable_resource_type_is_recognized_as_invalid() {
        let root = tempfile::tempdir().unwrap();
        let project = project(RepositoryLayout::Single);
        std::fs::create_dir_all(root.path().join("api/unknown")).unwrap();
        std::fs::write(
            root.path().join("api/unknown/.resource.yaml"),
            "schema_version: 1\nmetadata: { track: true }\n",
        )
        .unwrap();

        assert!(
            validate_target_tree(root.path(), &project, "dev", "api", &BTreeMap::new()).is_err()
        );
    }

    #[test]
    fn hint_beneath_an_unknown_target_is_invalid() {
        let root = tempfile::tempdir().unwrap();
        let mut project = project(RepositoryLayout::Single);
        project.environments.get_mut("dev").unwrap().targets.insert(
            "api".into(),
            crate::TargetConfig {
                application: "example".into(),
                url: "http://example.test".into(),
                from: None,
                auth: None,
                headers: BTreeMap::new(),
                sensitive_fields: BTreeMap::new(),
            },
        );
        std::fs::create_dir_all(root.path().join("typo/widgets")).unwrap();
        std::fs::write(
            root.path().join("typo/widgets/.resource.yaml"),
            "schema_version: 1\nmetadata: { track: true }\n",
        )
        .unwrap();

        assert!(validate_project_placement(root.path(), &project).is_err());
    }

    #[test]
    fn binding_material_distinguishes_absence_presence_and_raw_bytes() {
        let root = tempfile::tempdir().unwrap();
        let project = project(RepositoryLayout::Single);
        std::fs::create_dir_all(root.path().join("api/widgets")).unwrap();
        let absent = resolve(root.path(), &project, "dev", "api", None, "widgets")
            .unwrap()
            .binding_material(root.path())
            .unwrap();
        std::fs::write(
            root.path().join("api/.target.yaml"),
            "schema_version: 1\nmetadata: { track: false }\n",
        )
        .unwrap();
        let present = resolve(root.path(), &project, "dev", "api", None, "widgets")
            .unwrap()
            .binding_material(root.path())
            .unwrap();
        std::fs::write(
            root.path().join("api/.target.yaml"),
            "schema_version: 1\nmetadata:\n  track: false\n",
        )
        .unwrap();
        let reformatted = resolve(root.path(), &project, "dev", "api", None, "widgets")
            .unwrap()
            .binding_material(root.path())
            .unwrap();

        assert_ne!(absent, present);
        assert_ne!(present, reformatted);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_hint_is_rejected() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let project = project(RepositoryLayout::Single);
        std::fs::create_dir_all(root.path().join("api/widgets")).unwrap();
        std::fs::write(
            root.path().join("real.yaml"),
            "schema_version: 1\nmetadata: { track: true }\n",
        )
        .unwrap();
        symlink(
            root.path().join("real.yaml"),
            root.path().join("api/widgets/.resource.yaml"),
        )
        .unwrap();

        assert!(resolve(root.path(), &project, "dev", "api", None, "widgets").is_err());
    }
}
