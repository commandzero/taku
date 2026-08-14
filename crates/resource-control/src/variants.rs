use crate::application::load_installed;
use crate::provider::SecretFields;
use crate::transport::execute_probe;
use crate::{
    ApplicationDefinition, Operations, ResourceType, SCHEMA_VERSION, TargetConfig, git_root,
    load_project,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TargetBaseline {
    pub schema_version: u32,
    #[serde(default)]
    pub facts: BTreeMap<String, String>,
    #[serde(default)]
    pub variants: BTreeMap<String, String>,
}
pub struct ResolvedVariants {
    pub facts: BTreeMap<String, String>,
    pub selected: BTreeMap<String, String>,
    pub resource_types: BTreeMap<String, ResourceType>,
}

pub fn from_baseline(
    app: &ApplicationDefinition,
    target: &TargetConfig,
    baseline: &TargetBaseline,
) -> Result<ResolvedVariants> {
    let mut effective = BTreeMap::new();
    for (name, resource_type) in &app.target_profile.resource_types {
        let mut result = resource_type.clone();
        if !resource_type.variants.is_empty() {
            let selected = baseline
                .variants
                .get(name)
                .with_context(|| format!("Target Baseline has no selected Variant for {name}"))?;
            let variant = resource_type
                .variants
                .iter()
                .find(|variant| &variant.name == selected)
                .with_context(|| {
                    format!("Target Baseline selects unknown Variant {selected} for {name}")
                })?;
            if let Some(operations) = &variant.operations {
                merge_operations(&mut result.operations, operations);
            }
            result
                .transformations
                .extend(variant.transformations.clone());
            result.variants.clear();
        }
        add_target_sensitive_fields(name, target, &mut result)?;
        effective.insert(name.clone(), result);
    }
    Ok(ResolvedVariants {
        facts: baseline.facts.clone(),
        selected: baseline.variants.clone(),
        resource_types: effective,
    })
}

pub fn discover(
    app: &ApplicationDefinition,
    target: &TargetConfig,
    auth: &SecretFields,
) -> Result<ResolvedVariants> {
    let mut facts = BTreeMap::new();
    for probe in &app.target_profile.fact_probes {
        let value = execute_probe(target, app, &probe.operation, auth)?;
        let fact = value
            .pointer(&probe.pointer)
            .and_then(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .or_else(|| v.as_i64().map(|n| n.to_string()))
            })
            .with_context(|| {
                format!(
                    "Target Fact probe {} did not return {}",
                    probe.name, probe.pointer
                )
            })?;
        facts.insert(probe.name.clone(), fact);
    }
    let mut selected = BTreeMap::new();
    let mut effective = BTreeMap::new();
    for (name, rt) in &app.target_profile.resource_types {
        let mut result = rt.clone();
        if !rt.variants.is_empty() {
            let matching: Vec<_> = rt
                .variants
                .iter()
                .filter(|variant| {
                    variant
                        .facts
                        .iter()
                        .all(|(key, value)| facts.get(key) == Some(value))
                })
                .collect();
            if matching.len() != 1 {
                bail!(
                    "Resource Type {name} selected {} Variants; expected exactly one",
                    matching.len()
                );
            }
            let variant = matching[0];
            selected.insert(name.clone(), variant.name.clone());
            if let Some(operations) = &variant.operations {
                merge_operations(&mut result.operations, operations);
            }
            result
                .transformations
                .extend(variant.transformations.clone());
            result.variants.clear();
        }
        add_target_sensitive_fields(name, target, &mut result)?;
        effective.insert(name.clone(), result);
    }
    Ok(ResolvedVariants {
        facts,
        selected,
        resource_types: effective,
    })
}

fn add_target_sensitive_fields(
    name: &str,
    target: &TargetConfig,
    resource_type: &mut ResourceType,
) -> Result<()> {
    if let Some(pointers) = target.sensitive_fields.get(name) {
        for pointer in pointers {
            if !pointer.starts_with('/') {
                bail!("Target Sensitive Field for Resource Type {name} is not a JSON pointer");
            }
            if crate::model::sensitive_field_conflicts(resource_type, pointer) {
                bail!(
                    "Target Sensitive Field {pointer} overlaps required canonical state for Resource Type {name}"
                );
            }
            if !resource_type.sensitive_fields.contains(pointer) {
                resource_type.sensitive_fields.push(pointer.clone());
            }
        }
    }
    Ok(())
}

fn merge_operations(base: &mut Operations, overlay: &Operations) {
    if overlay.read.is_some() {
        base.read = overlay.read.clone();
    }
    if overlay.list.is_some() {
        base.list = overlay.list.clone();
    }
    if overlay.create.is_some() {
        base.create = overlay.create.clone();
    }
    if overlay.update.is_some() {
        base.update = overlay.update.clone();
    }
    if overlay.upsert.is_some() {
        base.upsert = overlay.upsert.clone();
    }
    if overlay.delete.is_some() {
        base.delete = overlay.delete.clone();
    }
}
pub fn baseline_path(root: &Path, environment: &str, target: &str) -> PathBuf {
    root.join(".taku/baselines")
        .join(environment)
        .join(format!("{target}.yml"))
}
pub fn load_baseline(path: &Path) -> Result<TargetBaseline> {
    let baseline: TargetBaseline = serde_yaml::from_str(&fs::read_to_string(path)?)?;
    if baseline.schema_version != SCHEMA_VERSION {
        bail!(
            "unsupported Target Baseline schema version {}",
            baseline.schema_version
        );
    }
    Ok(baseline)
}
pub fn save_baseline(path: &Path, baseline: &TargetBaseline) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_yaml::to_string(baseline)?)?;
    Ok(())
}

pub fn validate_project(root: &Path) -> Result<serde_json::Value> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let mut applications = Vec::new();
    for entry in fs::read_dir(root.join(".taku/applications"))
        .unwrap_or_else(|_| fs::read_dir(root.join(".taku")).unwrap())
    {
        let entry = entry?;
        if entry.path().join("resources.yml").is_file() {
            let name = entry.file_name().to_string_lossy().into_owned();
            load_installed(&root, &name)?;
            applications.push(name);
        }
    }
    for (environment, env) in &project.environments {
        for (target, target_config) in &env.targets {
            if !applications.contains(&target_config.application) {
                bail!(
                    "Target {environment}/{target} references uninstalled Application {}",
                    target_config.application
                );
            }
        }
    }
    Ok(
        serde_json::json!({"valid":true,"applications":applications,"environments":project.environments.len()}),
    )
}

pub fn baseline_from(
    facts: BTreeMap<String, String>,
    variants: BTreeMap<String, String>,
) -> TargetBaseline {
    TargetBaseline {
        schema_version: SCHEMA_VERSION,
        facts,
        variants,
    }
}
