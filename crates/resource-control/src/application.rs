use crate::{
    ApplicationDefinition, ApplicationSourceConfig, Installation, SCHEMA_VERSION, git_root,
    load_project,
};
use anyhow::{Context, Result, bail};
use rust_embed::Embed;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Embed)]
#[folder = "assets/applications/"]
#[include = "*/resources.yml"]
struct EmbeddedApplications;

#[derive(Clone, Debug, Serialize)]
pub struct ApplicationListing {
    pub name: String,
    pub version: String,
    pub installed: bool,
    pub selected_source: String,
    pub shadowed_sources: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct InstallResult {
    pub name: String,
    pub version: String,
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
    pub source: String,
    pub outcome: String,
    pub checksum: String,
}
type ApplicationCandidate = (
    ApplicationDefinition,
    Vec<u8>,
    String,
    Option<String>,
    Option<String>,
);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceMetadata {
    schema_version: u32,
    location: String,
    revision: String,
}

pub fn embedded_application(name: &str) -> Result<(ApplicationDefinition, Vec<u8>)> {
    let path = format!("{name}/resources.yml");
    let file =
        EmbeddedApplications::get(&path).with_context(|| format!("unknown Application {name}"))?;
    let bytes = file.data.into_owned();
    let definition = parse_definition(name, &bytes)?;
    Ok((definition, bytes))
}

pub fn load_installed(root: &Path, name: &str) -> Result<ApplicationDefinition> {
    let bytes = fs::read(
        root.join(".taku/applications")
            .join(name)
            .join("resources.yml"),
    )
    .with_context(|| format!("Application {name} is not installed"))?;
    parse_definition(name, &bytes)
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
            if entry.path().join("resources.yml").is_file() {
                names.insert(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    let installed_root = root.join(".taku/applications");
    if let Ok(entries) = fs::read_dir(&installed_root) {
        for entry in entries.flatten() {
            if entry.path().join("resources.yml").is_file() {
                names.insert(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    names
        .into_iter()
        .map(|name| {
            let installed = installed_root.join(&name).join("resources.yml").is_file();
            let cached_path = cached.join(&name).join("resources.yml");
            let cached_definition = if cached_path.exists() {
                Some(load_cached(&root, &name)?)
            } else {
                None
            };
            let embedded_definition = embedded_application(&name).ok();
            let definition = if installed {
                load_installed(&root, &name)?
            } else if let Some((definition, _)) = &cached_definition {
                definition.clone()
            } else {
                embedded_definition
                    .as_ref()
                    .with_context(|| format!("Application {name} has no valid definition"))?
                    .0
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
                version: definition.application.version,
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
            .join("resources.yml");
        if destination.exists() {
            bail!("Application {name} is already installed; use `taku update {name}`");
        }
        let (mut definition, bytes, source, source_identity, revision) =
            source_candidate(&root, name)?;
        let checksum = hex::encode(Sha256::digest(&bytes));
        definition.installation = Some(Installation {
            source: source.clone(),
            checksum: checksum.clone(),
            taku_version: env!("CARGO_PKG_VERSION").into(),
            source_identity,
            revision,
        });
        loaded.push((name, definition, checksum, source));
    }
    let mut results = Vec::new();
    for (name, definition, checksum, source) in loaded {
        let destination = root.join(".taku/applications").join(name);
        fs::create_dir_all(&destination)?;
        fs::write(
            destination.join("resources.yml"),
            serde_yaml::to_string(&definition)?,
        )?;
        results.push(InstallResult {
            name: name.clone(),
            version: definition.application.version,
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
                if entry.path().join("resources.yml").is_file() {
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
                    embedded_application(&name).ok().map(|(d, b)| {
                        (
                            d,
                            b,
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
            load_cached(&root, &name).ok().map(|(d, b)| {
                let metadata = source_metadata(&root).ok();
                (
                    d,
                    b,
                    String::from("git"),
                    metadata.as_ref().map(|m| m.0.clone()),
                    metadata.map(|m| m.1),
                )
            })
        };
        let Some((mut definition, bytes, source, source_identity, revision)) = candidate else {
            candidates.push((name, None));
            continue;
        };
        validate_resources_for(&root, &name, &definition)?;
        let checksum = hex::encode(Sha256::digest(&bytes));
        definition.installation = Some(Installation {
            source: source.clone(),
            checksum: checksum.clone(),
            taku_version: env!("CARGO_PKG_VERSION").into(),
            source_identity,
            revision,
        });
        candidates.push((name, Some((definition, source, checksum))));
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
            if let Some((definition, source, checksum)) = candidate {
                let destination = staging.join(&name).join("resources.yml");
                let temporary = destination.with_extension("next");
                fs::write(&temporary, serde_yaml::to_string(&definition)?)?;
                fs::rename(temporary, destination)?;
                out.push(UpdateResult {
                    name,
                    version: definition.application.version,
                    source,
                    outcome: "updated".into(),
                    checksum,
                });
            } else {
                out.push(UpdateResult {
                    name,
                    version: "".into(),
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
        let bytes = fs::read(
            temporary
                .join("applications")
                .join(name)
                .join("resources.yml"),
        )?;
        let definition = parse_definition(name, &bytes)?;
        Ok(Some((
            definition,
            bytes,
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
fn load_cached(root: &Path, name: &str) -> Result<(ApplicationDefinition, Vec<u8>)> {
    let bytes = fs::read(
        cache_root(root)
            .join("applications")
            .join(name)
            .join("resources.yml"),
    )?;
    let definition = parse_definition(name, &bytes)?;
    Ok((definition, bytes))
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
        .join("resources.yml");
    if cached_path.exists() {
        let (definition, bytes) = load_cached(root, name)?;
        let (location, revision) = source_metadata(root)?;
        return Ok((
            definition,
            bytes,
            "git".into(),
            Some(location),
            Some(revision),
        ));
    }
    let (definition, bytes) = embedded_application(name)?;
    Ok((definition, bytes, "embedded".into(), None, None))
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
        let definition_path = entry.path().join("resources.yml");
        let definition_metadata = fs::symlink_metadata(&definition_path)
            .with_context(|| format!("Application {name} has no resources.yml"))?;
        if definition_metadata.file_type().is_symlink() || !definition_metadata.is_file() {
            bail!("Application {name} definition is not a regular file");
        }
        parse_definition(&name, &fs::read(definition_path)?)?;
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
            for (type_name, installed_type) in &installed.target_profile.resource_types {
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
                let updated_type = definition
                    .target_profile
                    .resource_types
                    .get(type_name)
                    .with_context(|| {
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

pub fn parse_definition(expected_name: &str, bytes: &[u8]) -> Result<ApplicationDefinition> {
    let definition: ApplicationDefinition =
        serde_yaml::from_slice(bytes).context("invalid Application definition")?;
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
    if definition.target_profile.resource_types.is_empty() {
        bail!("Application {expected_name} has no Resource Types");
    }
    for (name, resource_type) in &definition.target_profile.resource_types {
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
        for dependency in &resource_type.dependencies {
            if !definition
                .target_profile
                .resource_types
                .contains_key(dependency)
            {
                bail!("Resource Type {name} depends on unknown Resource Type {dependency}");
            }
        }
        for pointer in &resource_type.sensitive_fields {
            if crate::model::sensitive_field_conflicts(resource_type, pointer) {
                bail!("Sensitive Field {pointer} overlaps required canonical state for {name}");
            }
        }
        let fact_names: BTreeSet<_> = definition
            .target_profile
            .fact_probes
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        let mut predicates = BTreeSet::new();
        for variant in &resource_type.variants {
            if let Some(operations) = &variant.operations {
                validate_operations(operations, name)?;
            }
            validate_transformations(&variant.transformations, name)?;
            let mut variant_resource_type = resource_type.clone();
            variant_resource_type
                .transformations
                .extend(variant.transformations.clone());
            for pointer in &variant_resource_type.sensitive_fields {
                if crate::model::sensitive_field_conflicts(&variant_resource_type, pointer) {
                    bail!(
                        "Sensitive Field {pointer} overlaps required Variant canonical state for {name}"
                    );
                }
            }
            for fact in variant.facts.keys() {
                if !fact_names.contains(fact.as_str()) {
                    bail!(
                        "Resource Type {name} Variant {} references unknown Target Fact {fact}",
                        variant.name
                    );
                }
            }
            let signature = serde_json::to_string(&variant.facts)?;
            if !predicates.insert(signature) {
                bail!("Resource Type {name} has ambiguous duplicate Variant predicates");
            }
            let write_available = match resource_type.write_intent {
                crate::WriteIntent::Create => variant
                    .operations
                    .as_ref()
                    .and_then(|operations| operations.create.as_ref())
                    .or(resource_type.operations.create.as_ref())
                    .is_some(),
                crate::WriteIntent::Update => variant
                    .operations
                    .as_ref()
                    .and_then(|operations| operations.update.as_ref())
                    .or(resource_type.operations.update.as_ref())
                    .is_some(),
                crate::WriteIntent::Upsert => {
                    let overlay = variant.operations.as_ref();
                    let upsert = overlay
                        .and_then(|operations| operations.upsert.as_ref())
                        .or(resource_type.operations.upsert.as_ref());
                    let create = overlay
                        .and_then(|operations| operations.create.as_ref())
                        .or(resource_type.operations.create.as_ref());
                    let update = overlay
                        .and_then(|operations| operations.update.as_ref())
                        .or(resource_type.operations.update.as_ref());
                    upsert.is_some() || (create.is_some() && update.is_some())
                }
            };
            if !write_available {
                bail!(
                    "Resource Type {name} Variant {} cannot enforce configured Write Intent",
                    variant.name
                );
            }
        }
        if resource_type.variants.is_empty()
            && match resource_type.write_intent {
                crate::WriteIntent::Create => resource_type.operations.create.is_none(),
                crate::WriteIntent::Update => resource_type.operations.update.is_none(),
                crate::WriteIntent::Upsert => {
                    resource_type.operations.upsert.is_none()
                        && (resource_type.operations.create.is_none()
                            || resource_type.operations.update.is_none())
                }
            }
        {
            bail!("Resource Type {name} cannot enforce configured Write Intent");
        }
    }
    for probe in &definition.target_profile.fact_probes {
        validate_operation(&probe.operation, &format!("Target Fact {}", probe.name))?;
    }
    validate_dependencies(&definition)?;
    Ok(definition)
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
    if operation
        .default_namespace_path
        .as_ref()
        .is_some_and(|path| !path.starts_with('/'))
    {
        bail!("{owner} default Namespace Operation path must begin with /");
    }
    if operation.namespace.is_some() && operation.default_namespace_path.is_some() {
        bail!("{owner} Operation cannot define both namespace wrappers and default_namespace_path");
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
    if matches!(
        operation.bundle.as_ref(),
        Some(crate::Bundle::Config(crate::BundleConfig {
            format: crate::PayloadFormat::MultipartNdjson,
            ..
        }))
    ) {
        bail!("{owner} Operation must configure multipart separately from bundle format");
    }
    if let Some(multipart) = operation.bundle.as_ref().and_then(crate::Bundle::multipart)
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

fn validate_dependencies(definition: &ApplicationDefinition) -> Result<()> {
    fn visit(
        name: &str,
        definition: &ApplicationDefinition,
        visiting: &mut BTreeSet<String>,
        done: &mut BTreeSet<String>,
    ) -> Result<()> {
        if done.contains(name) {
            return Ok(());
        }
        if !visiting.insert(name.into()) {
            bail!("Resource Type dependency cycle includes {name}");
        }
        for dep in &definition.target_profile.resource_types[name].dependencies {
            visit(dep, definition, visiting, done)?;
        }
        visiting.remove(name);
        done.insert(name.into());
        Ok(())
    }
    let mut visiting = BTreeSet::new();
    let mut done = BTreeSet::new();
    for name in definition.target_profile.resource_types.keys() {
        visit(name, definition, &mut visiting, &mut done)?;
    }
    Ok(())
}
