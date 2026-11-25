use super::catalog::validate_catalog;
use crate::{ApplicationDefinition, ResourceTypeCatalog, SCHEMA_VERSION};
use anyhow::{Context, Result, bail};
use rust_embed::Embed;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Embed)]
#[folder = "assets/applications/"]
#[include = "*/*.yaml"]
struct EmbeddedApplications;

#[derive(Clone)]
pub(super) struct ApplicationBundle {
    pub(super) definition: ApplicationDefinition,
    pub(super) files: BTreeMap<String, Vec<u8>>,
}

pub(super) fn embedded_application(name: &str) -> Result<ApplicationBundle> {
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

fn parse_bundle(
    expected_name: &str,
    files: BTreeMap<String, Vec<u8>>,
) -> Result<ApplicationBundle> {
    ApplicationFiles {
        name: expected_name,
        files,
    }
    .parse()
}

struct ApplicationFiles<'a> {
    name: &'a str,
    files: BTreeMap<String, Vec<u8>>,
}

impl ApplicationFiles<'_> {
    fn parse(self) -> Result<ApplicationBundle> {
        let manifest = self
            .files
            .get("application.yaml")
            .context("Application has no application.yaml")?;
        let mut definition: ApplicationDefinition =
            serde_yaml::from_slice(manifest).context("invalid Application definition")?;
        self.validate_manifest(&definition)?;
        definition.catalogs = self.read_catalogs()?;
        Ok(ApplicationBundle {
            definition,
            files: self.files,
        })
    }

    fn validate_manifest(&self, definition: &ApplicationDefinition) -> Result<()> {
        let expected_name = self.name;
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
            validate_endpoint(endpoint)?;
        }
        Ok(())
    }

    fn read_catalogs(&self) -> Result<BTreeMap<u64, ResourceTypeCatalog>> {
        let expected_name = self.name;
        let mut catalogs = BTreeMap::new();
        for (path, bytes) in self
            .files
            .iter()
            .filter(|(path, _)| *path != "application.yaml")
        {
            let major = version_file_major(path)
                .with_context(|| format!("unexpected Application definition file {path}"))?;
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
        Ok(catalogs)
    }
}

fn validate_endpoint(endpoint: &crate::VersionEndpoint) -> Result<()> {
    reqwest::Method::from_bytes(endpoint.method.as_bytes())
        .context("Version Endpoint has an invalid HTTP method")?;
    if !endpoint.path.starts_with('/') {
        bail!("Version Endpoint path must begin with /");
    }
    if !endpoint.pointer.starts_with('/') {
        bail!("Version Endpoint pointer must be a JSON pointer");
    }
    Ok(())
}

fn version_file_major(path: &str) -> Option<u64> {
    path.strip_prefix("version-")?
        .strip_suffix(".yaml")?
        .parse()
        .ok()
}

pub(super) fn read_bundle_directory(name: &str, directory: &Path) -> Result<ApplicationBundle> {
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
        if path == "application.yaml" || version_file_major(&path).is_some() {
            files.insert(path, fs::read(entry.path())?);
        } else {
            bail!("unexpected Application definition file {path}");
        }
    }
    parse_bundle(name, files)
}

pub(super) fn write_bundle(destination: &Path, bundle: &ApplicationBundle) -> Result<()> {
    fs::create_dir_all(destination)?;
    for (path, bytes) in &bundle.files {
        let bytes = if path == "application.yaml" {
            serde_yaml::to_string(&bundle.definition)?.into_bytes()
        } else {
            bytes.clone()
        };
        fs::write(destination.join(path), bytes)?;
    }
    Ok(())
}

pub(super) fn bundle_checksum(files: &BTreeMap<String, Vec<u8>>) -> String {
    let mut hasher = Sha256::new();
    for (path, bytes) in files {
        hasher.update((path.len() as u64).to_be_bytes());
        hasher.update(path.as_bytes());
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }
    hex::encode(hasher.finalize())
}

pub(super) fn supported_versions(definition: &ApplicationDefinition) -> Vec<String> {
    definition
        .catalogs
        .values()
        .map(|catalog| catalog.application.version.clone())
        .collect()
}

pub(super) fn embedded_names() -> impl Iterator<Item = String> {
    EmbeddedApplications::iter().filter_map(|path| path.split('/').next().map(str::to_owned))
}
