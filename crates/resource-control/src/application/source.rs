use super::ApplicationCandidate;
use super::RefreshResult;
use super::bundle::{ApplicationBundle, embedded_application, read_bundle_directory};
use crate::{ApplicationSourceConfig, SCHEMA_VERSION, git_root, load_project};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceMetadata {
    schema_version: u32,
    location: String,
    revision: String,
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
        temporary.join(".source.yaml"),
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

pub(super) fn load_application_from_git_source(
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

pub(super) fn cache_root(root: &Path) -> std::path::PathBuf {
    root.join(".taku/cache/application-source")
}
pub(super) fn load_cached(root: &Path, name: &str) -> Result<ApplicationBundle> {
    read_bundle_directory(name, &cache_root(root).join("applications").join(name))
}
pub(super) fn source_metadata(root: &Path) -> Result<(String, String)> {
    let value: SourceMetadata =
        serde_yaml::from_str(&fs::read_to_string(cache_root(root).join(".source.yaml"))?)?;
    if value.schema_version != SCHEMA_VERSION {
        bail!(
            "unsupported Application Source metadata schema version {}",
            value.schema_version
        );
    }
    Ok((value.location, value.revision))
}
pub(super) fn source_candidate(root: &Path, name: &str) -> Result<ApplicationCandidate> {
    let cached_path = cache_root(root)
        .join("applications")
        .join(name)
        .join("application.yaml");
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
        let definition_path = entry.path().join("application.yaml");
        let definition_metadata = fs::symlink_metadata(&definition_path)
            .with_context(|| format!("Application {name} has no application.yaml"))?;
        if definition_metadata.file_type().is_symlink() || !definition_metadata.is_file() {
            bail!("Application {name} definition is not a regular file");
        }
        read_bundle_directory(&name, &entry.path())?;
        names.push(name);
    }
    names.sort();
    Ok(names)
}
