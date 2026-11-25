use super::bundle::{
    ApplicationBundle, bundle_checksum, embedded_application, supported_versions, write_bundle,
};
use super::compatibility::validate_resources_for;
use super::source::{
    load_application_from_git_source, load_cached, refresh_source, source_metadata,
};
use super::{ApplicationCandidate, UpdateResult, load_installed, names_in};
use crate::{Installation, git_root};
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::Path;

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
    let selected = selected_applications(&installed_root, names)?;
    let candidates = selected
        .into_iter()
        .map(|name| prepare_candidate(&root, name, from.is_some()))
        .collect::<Result<Vec<_>>>()?;
    stage_update(&root, &installed_root, candidates)
}

fn selected_applications(installed_root: &Path, names: &[String]) -> Result<Vec<String>> {
    let selected = if names.is_empty() {
        let mut installed = names_in(installed_root).collect::<Vec<_>>();
        installed.sort();
        installed
    } else {
        names.to_vec()
    };
    if selected.is_empty() {
        bail!("no installed Applications selected");
    }
    Ok(selected)
}

struct PreparedUpdate {
    bundle: ApplicationBundle,
    source: String,
    checksum: String,
}

fn prepare_candidate(
    root: &Path,
    name: String,
    explicit_source: bool,
) -> Result<(String, Option<PreparedUpdate>)> {
    let installed = load_installed(root, &name)?;
    let candidate = if explicit_source {
        load_cached(root, &name).ok().map(|bundle| {
            let metadata = source_metadata(root).ok();
            (
                bundle,
                "git".into(),
                metadata.as_ref().map(|m| m.0.clone()),
                metadata.map(|m| m.1),
            )
        })
    } else {
        installed_candidate(root, &name, installed.installation.as_ref())?
    };
    let Some((mut bundle, source, source_identity, revision)) = candidate else {
        return Ok((name, None));
    };
    validate_resources_for(root, &name, &bundle.definition)?;
    let checksum = bundle_checksum(&bundle.files);
    bundle.definition.installation = Some(Installation {
        source: source.clone(),
        checksum: checksum.clone(),
        taku_version: env!("CARGO_PKG_VERSION").into(),
        source_identity,
        revision,
    });
    Ok((
        name,
        Some(PreparedUpdate {
            bundle,
            source,
            checksum,
        }),
    ))
}

fn installed_candidate(
    root: &Path,
    name: &str,
    installation: Option<&Installation>,
) -> Result<Option<ApplicationCandidate>> {
    let installation = installation.context("installed Application has no provenance")?;
    match installation.source.as_str() {
        "embedded" => Ok(embedded_application(name)
            .ok()
            .map(|bundle| (bundle, "embedded".into(), None, None))),
        "git" => {
            let source = installation
                .source_identity
                .as_deref()
                .context("installed Git Application has no source provenance")?;
            load_application_from_git_source(root, source, name)
        }
        source => bail!("unsupported installed Application source {source}"),
    }
}

fn stage_update(
    root: &Path,
    installed_root: &Path,
    candidates: Vec<(String, Option<PreparedUpdate>)>,
) -> Result<Vec<UpdateResult>> {
    let staging = root.join(".taku/applications.update.next");
    let backup = root.join(".taku/applications.update.previous");
    if staging.exists() || backup.exists() {
        bail!("an unfinished Application update transaction requires manual recovery");
    }
    if let Err(error) = copy_directory(installed_root, &staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    let staged = candidates
        .into_iter()
        .map(|(name, candidate)| write_update(&staging, name, candidate))
        .collect::<Result<Vec<_>>>();
    let out = match staged {
        Ok(out) => out,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
    };
    fs::rename(installed_root, &backup)?;
    if let Err(error) = fs::rename(&staging, installed_root) {
        fs::rename(&backup, installed_root).context(
            "Application update failed and rollback could not restore the installed set",
        )?;
        return Err(error.into());
    }
    fs::remove_dir_all(backup)?;
    Ok(out)
}

fn write_update(
    staging: &Path,
    name: String,
    candidate: Option<PreparedUpdate>,
) -> Result<UpdateResult> {
    let Some(PreparedUpdate {
        bundle,
        source,
        checksum,
    }) = candidate
    else {
        return Ok(UpdateResult {
            name,
            version: String::new(),
            definition_version: String::new(),
            source: String::new(),
            outcome: "skipped_absent".into(),
            checksum: String::new(),
        });
    };
    let destination = staging.join(&name);
    if destination.exists() {
        fs::remove_dir_all(&destination)?;
    }
    write_bundle(&destination, &bundle)?;
    Ok(UpdateResult {
        name,
        version: supported_versions(&bundle.definition).join(", "),
        definition_version: bundle.definition.version,
        source,
        outcome: "updated".into(),
        checksum,
    })
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
