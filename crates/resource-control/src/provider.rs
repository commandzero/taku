use crate::{Project, ProviderConfig, TargetConfig};
use anyhow::{Context, Result, bail};
use redact::Secret;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::path::Path;

pub type SecretFields = BTreeMap<String, Secret<String>>;

pub fn resolve_auth(
    root: &Path,
    project: &Project,
    environment: &str,
    target: &TargetConfig,
    cli: &BTreeMap<String, String>,
) -> Result<SecretFields> {
    let config = target
        .auth
        .as_ref()
        .or(project.environments[environment].provider.as_ref());
    let Some(config) = config else {
        return Ok(BTreeMap::new());
    };
    let file = read_dotenv(root, config)?;
    let mut result = BTreeMap::new();
    for (field, variable) in &config.fields {
        let value = cli
            .get(field)
            .cloned()
            .or_else(|| std::env::var(variable).ok())
            .or_else(|| file.get(variable).cloned())
            .with_context(|| format!("required provider value {field} is unavailable"))?;
        if value.is_empty() {
            bail!("required provider value {field} is empty");
        }
        result.insert(field.clone(), Secret::new(value));
    }
    Ok(result)
}

fn read_dotenv(root: &Path, config: &ProviderConfig) -> Result<BTreeMap<String, String>> {
    let Some(path) = &config.dotenv else {
        return Ok(BTreeMap::new());
    };
    let path = {
        let path = Path::new(path);
        if path.is_absolute() {
            path.to_owned()
        } else {
            root.join(path)
        }
    };
    let mut bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if config.optional && error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BTreeMap::new());
        }
        Err(_) => bail!("unable to read configured dotenv file {}", path.display()),
    };
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        bytes.drain(..3);
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| anyhow::anyhow!("configured dotenv file is not valid UTF-8"))?;
    let iterator = dotenvy::from_read_iter(Cursor::new(text));
    let mut result = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for entry in iterator {
        let (key, value) =
            entry.map_err(|_| anyhow::anyhow!("configured dotenv file contains invalid syntax"))?;
        if !seen.insert(key.clone()) {
            bail!("configured dotenv file contains duplicate key {key}");
        }
        result.insert(key, value);
    }
    Ok(result)
}
