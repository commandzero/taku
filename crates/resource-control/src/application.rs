use crate::{
    ApplicationDefinition, ApplicationSourceConfig, Installation, ResourceTypeCatalog,
    SCHEMA_VERSION, git_root, load_project,
};
use anyhow::{Context, Result, bail};
use rust_embed::Embed;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Embed)]
#[folder = "assets/applications/"]
#[include = "*/*.yml"]
struct EmbeddedApplications;

#[derive(Clone)]
struct ApplicationBundle {
    definition: ApplicationDefinition,
    files: BTreeMap<String, Vec<u8>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ApplicationListing {
    pub name: String,
    pub version: String,
    pub definition_version: String,
    pub installed: bool,
    pub selected_source: String,
    pub shadowed_sources: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct InstallResult {
    pub name: String,
    pub version: String,
    pub definition_version: String,
    pub source: String,
    pub checksum: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct RefreshResult {
    pub source: String,
    pub revision: String,
    pub applications: Vec<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct UpdateResult {
    pub name: String,
    pub version: String,
    pub definition_version: String,
    pub source: String,
    pub outcome: String,
    pub checksum: String,
}
type ApplicationCandidate = (ApplicationBundle, String, Option<String>, Option<String>);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceMetadata {
    schema_version: u32,
    location: String,
    revision: String,
}

fn embedded_application(name: &str) -> Result<ApplicationBundle> {
    let prefix = format!("{name}/");
    let files = EmbeddedApplications::iter()
        .filter_map(|path| {
            path.strip_prefix(&prefix).map(|relative| {
                EmbeddedApplications::get(path.as_ref())
                    .map(|file| (relative.to_owned(), file.data.into_owned()))
                    .with_context(|| format!("missing embedded Application file {path}"))
            })
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    if files.is_empty() {
        bail!("unknown Application {name}");
    }
    parse_bundle(name, files)
}

pub fn load_installed(root: &Path, name: &str) -> Result<ApplicationDefinition> {
    let path = root.join(".taku/applications").join(name);
    if !path.join("application.yml").is_file() {
        bail!("Application {name} is not installed");
    }
    Ok(read_bundle_directory(name, &path)?.definition)
}

pub fn list_applications(root: &Path) -> Result<Vec<ApplicationListing>> {
    let root = git_root(root)?;
    let mut names = BTreeSet::new();
    for path in EmbeddedApplications::iter() {
        if let Some(name) = path.split('/').next() {
            names.insert(name.to_owned());
        }
    }
    let cached = cache_root(&root).join("applications");
    if let Ok(entries) = fs::read_dir(&cached) {
        for entry in entries.flatten() {
            if entry.path().join("application.yml").is_file() {
                names.insert(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    let installed_root = root.join(".taku/applications");
    if let Ok(entries) = fs::read_dir(&installed_root) {
        for entry in entries.flatten() {
            if entry.path().join("application.yml").is_file() {
                names.insert(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    names
        .into_iter()
        .map(|name| {
            let installed = installed_root.join(&name).join("application.yml").is_file();
            let cached_path = cached.join(&name).join("application.yml");
            let cached_definition = if cached_path.exists() {
                Some(load_cached(&root, &name)?)
            } else {
                None
            };
            let embedded_definition = embedded_application(&name).ok();
            let definition = if installed {
                load_installed(&root, &name)?
            } else if let Some(bundle) = &cached_definition {
                bundle.definition.clone()
            } else {
                embedded_definition
                    .as_ref()
                    .with_context(|| format!("Application {name} has no valid definition"))?
                    .definition
                    .clone()
            };
            let selected_source = if installed {
                definition
                    .installation
                    .as_ref()
                    .map(|i| i.source.clone())
                    .unwrap_or_else(|| "unknown".into())
            } else if cached_definition.is_some() {
                "git".into()
            } else {
                "embedded".into()
            };
            let shadowed_sources =
                if !installed && cached_definition.is_some() && embedded_definition.is_some() {
                    vec!["embedded".into()]
                } else {
                    vec![]
                };
            Ok(ApplicationListing {
                name,
                version: supported_versions(&definition).join(", "),
                definition_version: definition.version,
                installed,
                selected_source,
                shadowed_sources,
            })
        })
        .collect()
}

pub fn install_applications(root: &Path, names: &[String]) -> Result<Vec<InstallResult>> {
    install_applications_from(root, names, None)
}

pub fn install_applications_from(
    root: &Path,
    names: &[String],
    from: Option<&str>,
) -> Result<Vec<InstallResult>> {
    let root = git_root(root)?;
    if from.is_some() {
        refresh_source(&root, from)?;
    }
    if names.is_empty() {
        bail!("at least one Application is required");
    }
    let mut loaded = Vec::new();
    let mut unique = BTreeSet::new();
    for name in names {
        if !unique.insert(name) {
            bail!("Application {name} was selected more than once");
        }
        let destination = root
            .join(".taku/applications")
            .join(name)
            .join("application.yml");
        if destination.exists() {
            bail!("Application {name} is already installed; use `taku update {name}`");
        }
        let (mut bundle, source, source_identity, revision) = source_candidate(&root, name)?;
        let checksum = bundle_checksum(&bundle.files);
        bundle.definition.installation = Some(Installation {
            source: source.clone(),
            checksum: checksum.clone(),
            taku_version: env!("CARGO_PKG_VERSION").into(),
            source_identity,
            revision,
        });
        loaded.push((name, bundle, checksum, source));
    }
    let mut results = Vec::new();
    for (name, bundle, checksum, source) in loaded {
        let destination = root.join(".taku/applications").join(name);
        write_bundle(&destination, &bundle)?;
        results.push(InstallResult {
            name: name.clone(),
            version: supported_versions(&bundle.definition).join(", "),
            definition_version: bundle.definition.version.clone(),
            source,
            checksum,
        });
    }
    Ok(results)
}

pub fn refresh_source(root: &Path, from: Option<&str>) -> Result<RefreshResult> {
    let root = git_root(root)?;
    let mut project = load_project(&root)?;
    let location = from
        .map(str::to_owned)
        .or_else(|| {
            project
                .application_source
                .as_ref()
                .map(|s| s.location.clone())
        })
        .context("no Git Application Source is configured")?;
    if from.is_some() {
        project.application_source = Some(ApplicationSourceConfig {
            location: location.clone(),
        });
        crate::project::save_project(&root, &project)?;
    }
    let parent = root.join(".taku/cache");
    fs::create_dir_all(&parent)?;
    let temporary = parent.join(format!("application-source.next-{}", std::process::id()));
    if temporary.exists() {
        fs::remove_dir_all(&temporary)?;
    }
    let output = Command::new("git")
        .args(["clone", "--quiet", "--depth", "1", "--", &location])
        .arg(&temporary)
        .output()
        .context("failed to run Git for Application Source")?;
    if !output.status.success() {
        let _ = fs::remove_dir_all(&temporary);
        bail!("failed to refresh Git Application Source");
    }
    let revision_output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&temporary)
        .output()?;
    if !revision_output.status.success() {
        let _ = fs::remove_dir_all(&temporary);
        bail!("refreshed Application Source has no resolved revision");
    }
    let revision = String::from_utf8_lossy(&revision_output.stdout)
        .trim()
        .to_owned();
    let applications = validate_source_tree(&temporary)?;
    fs::write(
        temporary.join(".source.yml"),
        serde_yaml::to_string(
            &serde_json::json!({"schema_version":1,"location":location,"revision":revision}),
        )?,
    )?;
    let final_path = cache_root(&root);
    if final_path.exists() {
        fs::remove_dir_all(&final_path)?;
    }
    fs::rename(temporary, &final_path)?;
    Ok(RefreshResult {
        source: location,
        revision,
        applications,
    })
}

pub fn update_applications(
    root: &Path,
    names: &[String],
    from: Option<&str>,
) -> Result<Vec<UpdateResult>> {
    let root = git_root(root)?;
    if from.is_some() {
        refresh_source(&root, from)?;
    }
    let installed_root = root.join(".taku/applications");
    let selected: Vec<String> = if names.is_empty() {
        let mut names = Vec::new();
        if let Ok(entries) = fs::read_dir(&installed_root) {
            for entry in entries.flatten() {
                if entry.path().join("application.yml").is_file() {
                    names.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
        }
        names.sort();
        names
    } else {
        names.to_vec()
    };
    if selected.is_empty() {
        bail!("no installed Applications selected");
    }
    let mut candidates = Vec::new();
    for name in selected {
        let installed = load_installed(&root, &name)?;
        let candidate = if from.is_none() {
            match installed.installation.as_ref() {
                Some(installation) if installation.source == "embedded" => {
                    embedded_application(&name).ok().map(|bundle| {
                        (
                            bundle,
                            String::from("embedded"),
                            None::<String>,
                            None::<String>,
                        )
                    })
                }
                Some(installation) if installation.source == "git" => {
                    let source = installation
                        .source_identity
                        .as_deref()
                        .context("installed Git Application has no source provenance")?;
                    load_application_from_git_source(&root, source, &name)?
                }
                Some(installation) => {
                    bail!(
                        "unsupported installed Application source {}",
                        installation.source
                    )
                }
                None => bail!("installed Application has no provenance"),
            }
        } else {
            load_cached(&root, &name).ok().map(|bundle| {
                let metadata = source_metadata(&root).ok();
                (
                    bundle,
                    String::from("git"),
                    metadata.as_ref().map(|m| m.0.clone()),
                    metadata.map(|m| m.1),
                )
            })
        };
        let Some((mut bundle, source, source_identity, revision)) = candidate else {
            candidates.push((name, None));
            continue;
        };
        validate_resources_for(&root, &name, &bundle.definition)?;
        let checksum = bundle_checksum(&bundle.files);
        bundle.definition.installation = Some(Installation {
            source: source.clone(),
            checksum: checksum.clone(),
            taku_version: env!("CARGO_PKG_VERSION").into(),
            source_identity,
            revision,
        });
        candidates.push((name, Some((bundle, source, checksum))));
    }
    let staging = root.join(".taku/applications.update.next");
    let backup = root.join(".taku/applications.update.previous");
    if staging.exists() || backup.exists() {
        bail!("an unfinished Application update transaction requires manual recovery");
    }
    if let Err(error) = copy_directory(&installed_root, &staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    let staged = (|| -> Result<Vec<UpdateResult>> {
        let mut out = Vec::new();
        for (name, candidate) in candidates {
            if let Some((bundle, source, checksum)) = candidate {
                let destination = staging.join(&name);
                if destination.exists() {
                    fs::remove_dir_all(&destination)?;
                }
                write_bundle(&destination, &bundle)?;
                out.push(UpdateResult {
                    name,
                    version: supported_versions(&bundle.definition).join(", "),
                    definition_version: bundle.definition.version.clone(),
                    source,
                    outcome: "updated".into(),
                    checksum,
                });
            } else {
                out.push(UpdateResult {
                    name,
                    version: "".into(),
                    definition_version: "".into(),
                    source: "".into(),
                    outcome: "skipped_absent".into(),
                    checksum: "".into(),
                });
            }
        }
        Ok(out)
    })();
    let out = match staged {
        Ok(out) => out,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    fs::rename(&installed_root, &backup)?;
    if let Err(error) = fs::rename(&staging, &installed_root) {
        fs::rename(&backup, &installed_root).context(
            "Application update failed and rollback could not restore the installed set",
        )?;
        return Err(error.into());
    }
    fs::remove_dir_all(backup)?;
    Ok(out)
}

fn load_application_from_git_source(
    root: &Path,
    location: &str,
    name: &str,
) -> Result<Option<ApplicationCandidate>> {
    let suffix = &hex::encode(Sha256::digest(format!("{location}\0{name}").as_bytes()))[..12];
    let temporary = root.join(".taku/cache").join(format!(
        "application-update.next-{}-{suffix}",
        std::process::id()
    ));
    if temporary.exists() {
        fs::remove_dir_all(&temporary)?;
    }
    let loaded = (|| -> Result<Option<ApplicationCandidate>> {
        let output = Command::new("git")
            .args(["clone", "--quiet", "--depth", "1", "--", location])
            .arg(&temporary)
            .output()
            .context("failed to run Git for installed Application source")?;
        if !output.status.success() {
            bail!("failed to refresh installed Application source");
        }
        let applications = validate_source_tree(&temporary)?;
        if !applications.iter().any(|application| application == name) {
            return Ok(None);
        }
        let revision_output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&temporary)
            .output()?;
        if !revision_output.status.success() {
            bail!("installed Application source has no resolved revision");
        }
        let revision = String::from_utf8_lossy(&revision_output.stdout)
            .trim()
            .to_owned();
        let bundle = read_bundle_directory(name, &temporary.join("applications").join(name))?;
        Ok(Some((
            bundle,
            "git".into(),
            Some(location.into()),
            Some(revision),
        )))
    })();
    let _ = fs::remove_dir_all(&temporary);
    loaded
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() {
            bail!(
                "symlinked installed Application input is not allowed: {}",
                source_path.display()
            );
        }
        if metadata.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            fs::copy(&source_path, &destination_path)?;
        } else {
            bail!(
                "installed Application input is not a regular file: {}",
                source_path.display()
            );
        }
    }
    Ok(())
}

fn cache_root(root: &Path) -> std::path::PathBuf {
    root.join(".taku/cache/application-source")
}
fn load_cached(root: &Path, name: &str) -> Result<ApplicationBundle> {
    read_bundle_directory(name, &cache_root(root).join("applications").join(name))
}
fn source_metadata(root: &Path) -> Result<(String, String)> {
    let value: SourceMetadata =
        serde_yaml::from_str(&fs::read_to_string(cache_root(root).join(".source.yml"))?)?;
    if value.schema_version != SCHEMA_VERSION {
        bail!(
            "unsupported Application Source metadata schema version {}",
            value.schema_version
        );
    }
    Ok((value.location, value.revision))
}
fn source_candidate(root: &Path, name: &str) -> Result<ApplicationCandidate> {
    let cached_path = cache_root(root)
        .join("applications")
        .join(name)
        .join("application.yml");
    if cached_path.exists() {
        let bundle = load_cached(root, name)?;
        let (location, revision) = source_metadata(root)?;
        return Ok((bundle, "git".into(), Some(location), Some(revision)));
    }
    let bundle = embedded_application(name)?;
    Ok((bundle, "embedded".into(), None, None))
}
fn validate_source_tree(root: &Path) -> Result<Vec<String>> {
    let applications = root.join("applications");
    let mut names = Vec::new();
    for entry in
        fs::read_dir(&applications).context("Application Source has no applications directory")?
    {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            bail!("Application Source contains a symlink or special entry");
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let definition_path = entry.path().join("application.yml");
        let definition_metadata = fs::symlink_metadata(&definition_path)
            .with_context(|| format!("Application {name} has no application.yml"))?;
        if definition_metadata.file_type().is_symlink() || !definition_metadata.is_file() {
            bail!("Application {name} definition is not a regular file");
        }
        read_bundle_directory(&name, &entry.path())?;
        names.push(name);
    }
    names.sort();
    Ok(names)
}
fn validate_resources_for(
    root: &Path,
    name: &str,
    definition: &ApplicationDefinition,
) -> Result<()> {
    let project = load_project(root)?;
    let Some(installed) = load_installed(root, name).ok() else {
        return Ok(());
    };
    for (environment, env) in &project.environments {
        for (target_name, target) in &env.targets {
            if target.application != name {
                continue;
            }
            let baseline = crate::resolution::load_baseline(&crate::resolution::baseline_path(
                root,
                environment,
                target_name,
            ))
            .ok();
            let installed =
                crate::resolution::for_local_use(&installed, target, baseline.as_ref())?;
            let updated = crate::resolution::for_local_use(definition, target, baseline.as_ref())?;
            for (type_name, installed_type) in &installed.resource_types {
                let directories = crate::canonical::resource_directories(
                    root,
                    &project,
                    environment,
                    target_name,
                    type_name,
                    installed_type.namespaced,
                )?;
                if directories.is_empty() {
                    continue;
                }
                let updated_type = updated.resource_types.get(type_name).with_context(|| {
                    format!("updated Application removes managed Resource Type {type_name}")
                })?;
                if updated_type.namespaced != installed_type.namespaced {
                    bail!(
                        "updated Application changes namespace layout for managed Resource Type {type_name}"
                    );
                }
                if serde_yaml::to_string(&updated_type.filesystem)?
                    != serde_yaml::to_string(&installed_type.filesystem)?
                {
                    bail!(
                        "updated Application changes filesystem projection for managed Resource Type {type_name}"
                    );
                }
                for (_, directory) in directories {
                    for entry in fs::read_dir(directory)? {
                        let path = entry?.path();
                        let metadata = fs::symlink_metadata(&path)?;
                        if metadata.file_type().is_symlink() {
                            bail!(
                                "symlinked Resource input is not allowed: {}",
                                path.display()
                            );
                        }
                        let value = if let Some(projection) = &updated_type.filesystem {
                            if !metadata.is_dir() {
                                continue;
                            }
                            crate::projection::merge(&path, projection)?
                        } else {
                            if !metadata.is_file()
                                || !path
                                    .extension()
                                    .and_then(|extension| extension.to_str())
                                    .is_some_and(|extension| {
                                        matches!(extension, "json" | "json5" | "yaml" | "yml")
                                    })
                                || path
                                    .file_name()
                                    .and_then(|value| value.to_str())
                                    .is_some_and(|value| value.ends_with(".delete.yml"))
                            {
                                continue;
                            }
                            crate::canonical::parse_resource(&path)?
                        };
                        if crate::canonical::pointer_string(&value, &updated_type.id.pointer)
                            .is_none()
                        {
                            bail!(
                                "Resource {} is incompatible with updated Application",
                                path.display()
                            );
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn parse_bundle(
    expected_name: &str,
    files: BTreeMap<String, Vec<u8>>,
) -> Result<ApplicationBundle> {
    let manifest = files
        .get("application.yml")
        .context("Application has no application.yml")?;
    let mut definition: ApplicationDefinition =
        serde_yaml::from_slice(manifest).context("invalid Application definition")?;
    if definition.schema_version != SCHEMA_VERSION {
        bail!(
            "unsupported Application schema version {}",
            definition.schema_version
        );
    }
    if definition.application.name != expected_name {
        bail!(
            "Application identity {} does not match {expected_name}",
            definition.application.name
        );
    }
    if definition.version.is_empty() {
        bail!("Application {expected_name} has an empty definition version");
    }
    if definition.version_endpoints.is_empty() {
        bail!("Application {expected_name} has no Version Endpoints");
    }
    for endpoint in &definition.version_endpoints {
        reqwest::Method::from_bytes(endpoint.method.as_bytes())
            .context("Version Endpoint has an invalid HTTP method")?;
        if !endpoint.path.starts_with('/') {
            bail!("Version Endpoint path must begin with /");
        }
        if !endpoint.pointer.starts_with('/') {
            bail!("Version Endpoint pointer must be a JSON pointer");
        }
    }
    let mut catalogs = BTreeMap::new();
    for (path, bytes) in &files {
        let Some(major) = version_file_major(path) else {
            if path != "application.yml" {
                bail!("unexpected Application definition file {path}");
            }
            continue;
        };
        let catalog: ResourceTypeCatalog = serde_yaml::from_slice(bytes)
            .with_context(|| format!("invalid Major Version Catalog {path}"))?;
        validate_catalog(expected_name, major, &catalog)?;
        if catalogs.insert(major, catalog).is_some() {
            bail!("Application {expected_name} defines major version {major} more than once");
        }
    }
    if catalogs.is_empty() {
        bail!("Application {expected_name} has no Major Version Catalogs");
    }
    definition.catalogs = catalogs;
    Ok(ApplicationBundle { definition, files })
}

fn validate_catalog(expected_name: &str, major: u64, catalog: &ResourceTypeCatalog) -> Result<()> {
    if catalog.schema_version != SCHEMA_VERSION {
        bail!(
            "unsupported Major Version Catalog schema version {}",
            catalog.schema_version
        );
    }
    if catalog.version.is_empty() {
        bail!("Major Version Catalog {major} has an empty definition version");
    }
    if catalog.application.name != expected_name {
        bail!(
            "Major Version Catalog identity {} does not match {expected_name}",
            catalog.application.name
        );
    }
    semver::VersionReq::parse(&catalog.application.version).with_context(|| {
        format!(
            "Major Version Catalog {major} has invalid Application Version constraint {}",
            catalog.application.version
        )
    })?;
    if catalog.resource_types.is_empty() {
        bail!("Major Version Catalog {major} has no Resource Types");
    }
    for (name, definitions) in &catalog.resource_types {
        if definitions.is_empty() {
            bail!("Resource Type {name} has no definitions");
        }
        for resource_type in definitions {
            if let Some(version) = &resource_type.version {
                semver::VersionReq::parse(version).with_context(|| {
                    format!("Resource Type {name} has invalid version constraint {version}")
                })?;
            }
            validate_resource_type(resource_type, name)?;
            for dependency in &resource_type.dependencies {
                if !catalog.resource_types.contains_key(dependency) {
                    bail!("Resource Type {name} depends on unknown Resource Type {dependency}");
                }
            }
        }
    }
    Ok(())
}

fn validate_resource_type(resource_type: &crate::ResourceType, name: &str) -> Result<()> {
    if !resource_type.id.pointer.starts_with('/') {
        bail!("Resource Type {name} has an invalid identity pointer");
    }
    if resource_type.display_name.pointer.is_some()
        && !resource_type.display_name.pointers.is_empty()
    {
        bail!("Resource Type {name} Display Name cannot define both pointer and pointers");
    }
    if resource_type
        .display_name
        .pointers()
        .any(|pointer| !pointer.starts_with('/'))
    {
        bail!("Resource Type {name} has an invalid Display Name pointer");
    }
    if resource_type.operations.read.is_none() && resource_type.operations.list.is_none() {
        bail!("Resource Type {name} has no observation Operation");
    }
    validate_operations(&resource_type.operations, name)?;
    validate_transformations(&resource_type.transformations, name)?;
    validate_filesystem(resource_type, name)?;
    validate_metadata(resource_type, name)?;
    for pointer in &resource_type.sensitive_fields {
        if crate::model::sensitive_field_conflicts(resource_type, pointer) {
            bail!("Sensitive Field {pointer} overlaps required canonical state for {name}");
        }
    }
    if match resource_type.write_intent {
        crate::WriteIntent::Create => resource_type.operations.create.is_none(),
        crate::WriteIntent::Update => resource_type.operations.update.is_none(),
        crate::WriteIntent::Upsert => {
            resource_type.operations.upsert.is_none()
                && (resource_type.operations.create.is_none()
                    || resource_type.operations.update.is_none())
        }
    } {
        bail!("Resource Type {name} cannot enforce configured Write Intent");
    }
    Ok(())
}

fn pointers_overlap(left: &str, right: &str) -> bool {
    left == right
        || left.starts_with(&format!("{right}/"))
        || right.starts_with(&format!("{left}/"))
}

fn valid_json_pointer(pointer: &str) -> bool {
    if !pointer.starts_with('/') {
        return false;
    }
    let mut chars = pointer.chars();
    while let Some(character) = chars.next() {
        if character == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return false;
        }
    }
    true
}

fn validate_metadata(resource_type: &crate::ResourceType, name: &str) -> Result<()> {
    let Some(metadata) = &resource_type.metadata else {
        return Ok(());
    };
    if metadata.fields.is_empty() {
        bail!("Resource Type {name} metadata fields cannot be empty");
    }
    for (index, pointer) in metadata.fields.iter().enumerate() {
        if !valid_json_pointer(pointer) {
            bail!(
                "Resource Type {name} metadata field {pointer:?} is not a canonical JSON pointer"
            );
        }
        if metadata.fields[..index]
            .iter()
            .any(|other| pointers_overlap(pointer, other))
        {
            bail!("Resource Type {name} metadata fields overlap at {pointer}");
        }
        if pointers_overlap(pointer, &resource_type.id.pointer)
            || resource_type
                .display_name
                .pointers()
                .any(|required| pointers_overlap(pointer, required))
            || resource_type
                .sensitive_fields
                .iter()
                .any(|sensitive| pointers_overlap(pointer, sensitive))
        {
            bail!(
                "Resource Type {name} metadata field {pointer} overlaps required or sensitive canonical state"
            );
        }
        if resource_type
            .transformations
            .iter()
            .any(|transformation| match transformation {
                crate::Transformation::Extract { pointer: other }
                | crate::Transformation::Remove { pointer: other }
                | crate::Transformation::Omit { pointer: other }
                | crate::Transformation::Insert { pointer: other, .. }
                | crate::Transformation::EmbeddedJson { pointer: other }
                | crate::Transformation::Frame { pointer: other } => {
                    pointers_overlap(pointer, other)
                }
                crate::Transformation::SingletonMap {
                    pointer: other,
                    key_pointer,
                    value_pointer,
                } => {
                    pointers_overlap(pointer, other)
                        || pointers_overlap(pointer, key_pointer)
                        || pointers_overlap(pointer, value_pointer)
                }
            })
        {
            bail!("Resource Type {name} metadata field {pointer} overlaps a Transformation");
        }
        if let Some(frontmatter) = resource_type
            .filesystem
            .as_ref()
            .and_then(|filesystem| filesystem.frontmatter_markdown.as_ref())
        {
            let referenced = &frontmatter.referenced_files;
            if [
                &frontmatter.body_pointer,
                &referenced.pointer,
                &referenced.path_pointer,
                &referenced.name_pointer,
                &referenced.content_pointer,
            ]
            .into_iter()
            .any(|structural| pointers_overlap(pointer, structural))
            {
                bail!(
                    "Resource Type {name} metadata field {pointer} overlaps filesystem projection state"
                );
            }
        }
    }
    Ok(())
}

fn validate_operations(operations: &crate::Operations, owner: &str) -> Result<()> {
    for operation in [
        operations.read.as_ref(),
        operations.list.as_ref(),
        operations.create.as_ref(),
        operations.update.as_ref(),
        operations.upsert.as_ref(),
        operations.delete.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        validate_operation(operation, owner)?;
    }
    Ok(())
}

fn validate_filesystem(resource_type: &crate::ResourceType, owner: &str) -> Result<()> {
    let Some(filesystem) = &resource_type.filesystem else {
        return Ok(());
    };
    if filesystem.split != filesystem.merge {
        bail!("Resource Type {owner} split and merge formats must match");
    }
    let frontmatter = filesystem
        .frontmatter_markdown
        .as_ref()
        .context("frontmatter_markdown filesystem format requires its configuration")?;
    let document = Path::new(&frontmatter.document);
    if document.is_absolute()
        || document.components().count() != 1
        || frontmatter.document.is_empty()
    {
        bail!("Resource Type {owner} frontmatter document must be one relative path segment");
    }
    for (label, pointer) in [
        ("body_pointer", &frontmatter.body_pointer),
        (
            "referenced_files.pointer",
            &frontmatter.referenced_files.pointer,
        ),
        (
            "referenced_files.path_pointer",
            &frontmatter.referenced_files.path_pointer,
        ),
        (
            "referenced_files.name_pointer",
            &frontmatter.referenced_files.name_pointer,
        ),
        (
            "referenced_files.content_pointer",
            &frontmatter.referenced_files.content_pointer,
        ),
    ] {
        if !pointer.starts_with('/') {
            bail!("Resource Type {owner} {label} must be a JSON pointer");
        }
    }
    let extension = &frontmatter.referenced_files.extension;
    if extension.is_empty()
        || extension.starts_with('.')
        || extension.contains('/')
        || extension.contains('\\')
    {
        bail!("Resource Type {owner} referenced file extension is invalid");
    }
    Ok(())
}

fn validate_operation(operation: &crate::Operation, owner: &str) -> Result<()> {
    reqwest::Method::from_bytes(operation.method.as_bytes())
        .with_context(|| format!("{owner} has an invalid HTTP method"))?;
    if !operation.path.starts_with('/') {
        bail!("{owner} Operation path must begin with /");
    }
    if let Some(namespace) = &operation.namespace {
        if namespace.prefix.is_none() && namespace.suffix.is_none() {
            bail!("{owner} Operation namespace must define a prefix or suffix");
        }
        if namespace
            .prefix
            .as_ref()
            .is_some_and(|prefix| !prefix.starts_with('/'))
        {
            bail!("{owner} Operation namespace prefix must begin with /");
        }
        if namespace
            .suffix
            .as_ref()
            .is_some_and(|suffix| !suffix.starts_with('/'))
        {
            bail!("{owner} Operation namespace suffix must begin with /");
        }
        if !namespace
            .prefix
            .iter()
            .chain(namespace.suffix.iter())
            .any(|part| part.contains("{namespace}"))
        {
            bail!("{owner} Operation namespace prefix or suffix must contain {{namespace}}");
        }
    }
    if let Some(pointer) = &operation.body_pointer
        && (operation.body.is_none() || !pointer.starts_with('/'))
    {
        bail!("{owner} Operation body_pointer requires a body and a JSON pointer");
    }
    if operation
        .extract_missing
        .is_some_and(|outcome| matches!(outcome, crate::Outcome::Success))
    {
        bail!("{owner} Operation extract_missing cannot map to success");
    }
    if let Some(multipart) = operation
        .bundle
        .as_ref()
        .and_then(|bundle| bundle.multipart.as_ref())
        && (multipart.name.is_empty()
            || multipart.filename.is_empty()
            || multipart.content_type.is_empty())
    {
        bail!("{owner} Operation multipart bundle fields cannot be empty");
    }
    validate_transformations(&operation.transformations, owner)?;
    Ok(())
}

fn validate_transformations(transformations: &[crate::Transformation], owner: &str) -> Result<()> {
    for transformation in transformations {
        let pointers: Vec<&str> = match transformation {
            crate::Transformation::Extract { pointer }
            | crate::Transformation::Remove { pointer }
            | crate::Transformation::Omit { pointer }
            | crate::Transformation::Insert { pointer, .. }
            | crate::Transformation::EmbeddedJson { pointer }
            | crate::Transformation::Frame { pointer } => vec![pointer],
            crate::Transformation::SingletonMap {
                pointer,
                key_pointer,
                value_pointer,
            } => vec![pointer, key_pointer, value_pointer],
        };
        if pointers.iter().any(|pointer| !pointer.starts_with('/')) {
            bail!("{owner} Transformation has an invalid JSON pointer");
        }
        if let crate::Transformation::SingletonMap {
            key_pointer,
            value_pointer,
            ..
        } = transformation
            && (key_pointer == value_pointer
                || key_pointer.starts_with(&format!("{value_pointer}/"))
                || value_pointer.starts_with(&format!("{key_pointer}/")))
        {
            bail!("{owner} Singleton Map output pointers overlap");
        }
    }
    Ok(())
}

fn version_file_major(path: &str) -> Option<u64> {
    path.strip_prefix("version-")?
        .strip_suffix(".yml")?
        .parse()
        .ok()
}

fn read_bundle_directory(name: &str, directory: &Path) -> Result<ApplicationBundle> {
    let mut files = BTreeMap::new();
    for entry in
        fs::read_dir(directory).with_context(|| format!("Application {name} is not installed"))?
    {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            bail!("Application {name} contains a symlink or special entry");
        }
        let path = entry.file_name().to_string_lossy().into_owned();
        if path == "application.yml" || version_file_major(&path).is_some() {
            files.insert(path, fs::read(entry.path())?);
        } else {
            bail!("unexpected Application definition file {path}");
        }
    }
    parse_bundle(name, files)
}

fn write_bundle(destination: &Path, bundle: &ApplicationBundle) -> Result<()> {
    fs::create_dir_all(destination)?;
    for (path, bytes) in &bundle.files {
        let bytes = if path == "application.yml" {
            serde_yaml::to_string(&bundle.definition)?.into_bytes()
        } else {
            bytes.clone()
        };
        fs::write(destination.join(path), bytes)?;
    }
    Ok(())
}

fn bundle_checksum(files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut hasher = Sha256::new();
    for (path, bytes) in files {
        hasher.update((path.len() as u64).to_be_bytes());
        hasher.update(path.as_bytes());
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }
    hex::encode(hasher.finalize())
}

fn supported_versions(definition: &ApplicationDefinition) -> Vec<String> {
    definition
        .catalogs
        .values()
        .map(|catalog| catalog.application.version.clone())
        .collect()
}

#[cfg(test)]
mod metadata_validation_tests {
    use super::validate_resource_type;

    fn resource_type(extra: &str) -> crate::ResourceType {
        serde_yaml::from_str(&format!(
            r#"
id: {{ pointer: /id, scope: universal }}
display_name: {{ strategy: id }}
{extra}
operations:
  read: {{ method: GET, path: "/items/{{id}}", cardinality: one }}
  upsert: {{ method: PUT, path: "/items/{{id}}", cardinality: one }}
"#
        ))
        .unwrap()
    }

    #[test]
    fn accepts_distinct_canonical_metadata_pointers() {
        validate_resource_type(
            &resource_type("metadata: { fields: [/created_by, /updated_at] }"),
            "item",
        )
        .unwrap();
    }

    #[test]
    fn rejects_invalid_or_ambiguous_metadata_declarations() {
        for extra in [
            "metadata: { fields: [] }",
            "metadata: { fields: [''] }",
            "metadata: { fields: [/bad~2escape] }",
            "metadata: { fields: [/audit, /audit] }",
            "metadata: { fields: [/audit, /audit/user] }",
            "metadata: { fields: [/id] }",
            "metadata: { fields: [/name] }",
            "sensitive_fields: [/secret]\nmetadata: { fields: [/secret/owner] }",
            "transformations: [{ kind: remove, pointer: /server }]\nmetadata: { fields: [/server/time] }",
        ] {
            assert!(
                validate_resource_type(&resource_type(extra), "item").is_err(),
                "{extra}"
            );
        }
    }
}
