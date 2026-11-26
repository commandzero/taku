mod bundle;
mod catalog;
mod compatibility;
mod source;
mod update;

pub use source::refresh_source;
pub use update::update_applications;

use crate::{ApplicationDefinition, Installation, git_root};
use anyhow::{Context, Result, bail};
use bundle::{
    ApplicationBundle, bundle_checksum, embedded_application, embedded_names,
    read_bundle_directory, supported_versions, write_bundle,
};
use serde::Serialize;
use source::{cache_root, load_cached, source_candidate};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

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

pub fn load_installed(root: &Path, name: &str) -> Result<ApplicationDefinition> {
    let path = root.join(".taku/applications").join(name);
    if !path.join("application.yaml").is_file() {
        bail!("Application {name} is not installed");
    }
    Ok(read_bundle_directory(name, &path)?.definition)
}

pub fn list_applications(root: &Path) -> Result<Vec<ApplicationListing>> {
    let root = git_root(root)?;
    let cached = cache_root(&root).join("applications");
    let installed_root = root.join(".taku/applications");
    let names: BTreeSet<_> = embedded_names()
        .chain(names_in(&cached))
        .chain(names_in(&installed_root))
        .collect();
    names
        .into_iter()
        .map(|name| application_listing(&root, &cached, &installed_root, name))
        .collect()
}

fn names_in(directory: &Path) -> impl Iterator<Item = String> {
    fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().join("application.yaml").is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
}

fn application_listing(
    root: &Path,
    cached: &Path,
    installed_root: &Path,
    name: String,
) -> Result<ApplicationListing> {
    let installed = installed_root
        .join(&name)
        .join("application.yaml")
        .is_file();
    let cached_definition = if cached.join(&name).join("application.yaml").exists() {
        Some(load_cached(root, &name)?.definition)
    } else {
        None
    };
    let embedded_definition = embedded_application(&name)
        .ok()
        .map(|bundle| bundle.definition);
    let shadowed = !installed && cached_definition.is_some() && embedded_definition.is_some();
    let (definition, selected_source) = if installed {
        let definition = load_installed(root, &name)?;
        let source = definition
            .installation
            .as_ref()
            .map(|i| i.source.clone())
            .unwrap_or_else(|| "unknown".into());
        (definition, source)
    } else if let Some(definition) = cached_definition {
        (definition, "git".into())
    } else {
        (
            embedded_definition
                .with_context(|| format!("Application {name} has no valid definition"))?,
            "embedded".into(),
        )
    };
    Ok(ApplicationListing {
        name,
        version: supported_versions(&definition).join(", "),
        definition_version: definition.version,
        installed,
        selected_source,
        shadowed_sources: if shadowed {
            vec!["embedded".into()]
        } else {
            Vec::new()
        },
    })
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
            .join("application.yaml");
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
