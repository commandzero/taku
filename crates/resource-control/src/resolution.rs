use crate::application::load_installed;
use crate::provider::SecretFields;
use crate::transport::execute_version_endpoint;
use crate::{
    ApplicationDefinition, ResourceType, ResourceTypeCatalog, SCHEMA_VERSION, TargetConfig,
    git_root, load_project,
};
use anyhow::{Context, Result, bail};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TargetBaseline {
    pub schema_version: u32,
    pub application_version: String,
    pub catalog_version: String,
    #[serde(default)]
    pub definitions: BTreeMap<String, String>,
}

pub struct ResolvedApplication {
    pub application_version: String,
    pub catalog_version: String,
    pub selected: BTreeMap<String, String>,
    pub resource_types: BTreeMap<String, ResourceType>,
}

pub fn from_baseline(
    app: &ApplicationDefinition,
    target: &TargetConfig,
    baseline: &TargetBaseline,
) -> Result<ResolvedApplication> {
    let resolved = resolve_version(app, target, &baseline.application_version)?;
    if resolved.catalog_version != baseline.catalog_version {
        bail!(
            "Target Baseline catalog version {} does not match installed catalog version {}",
            baseline.catalog_version,
            resolved.catalog_version
        );
    }
    if resolved.selected != baseline.definitions {
        bail!("Target Baseline Resource Type Definitions do not match the installed Application");
    }
    Ok(resolved)
}

pub fn for_local_use(
    app: &ApplicationDefinition,
    target: &TargetConfig,
    baseline: Option<&TargetBaseline>,
) -> Result<ResolvedApplication> {
    if let Some(baseline) = baseline {
        return from_baseline(app, target, baseline);
    }
    if app.catalogs.len() != 1 {
        bail!(
            "Target Baseline is required to select a Major Version Catalog; run `taku fetch` or `taku list --remote`"
        );
    }
    let catalog = app.catalogs.values().next().unwrap();
    let mut resource_types = BTreeMap::new();
    let mut selected = BTreeMap::new();
    for (name, definitions) in &catalog.resource_types {
        if definitions.len() != 1 {
            bail!(
                "Target Baseline is required to select a Resource Type Definition for {name}; run `taku fetch` or `taku list --remote`"
            );
        }
        let mut definition = definitions[0].clone();
        let constraint = definition
            .version
            .clone()
            .unwrap_or_else(|| catalog.application.version.clone());
        definition.version = Some(constraint.clone());
        add_target_sensitive_fields(name, target, &mut definition)?;
        selected.insert(name.clone(), constraint);
        resource_types.insert(name.clone(), definition);
    }
    for (name, resource_type) in &resource_types {
        for dependency in &resource_type.dependencies {
            if !resource_types.contains_key(dependency) {
                bail!("Resource Type {name} depends on unavailable Resource Type {dependency}");
            }
        }
    }
    validate_dependency_cycles(&resource_types)?;
    Ok(ResolvedApplication {
        application_version: catalog.application.version.clone(),
        catalog_version: catalog.version.clone(),
        selected,
        resource_types,
    })
}

pub fn discover(
    app: &ApplicationDefinition,
    target: &TargetConfig,
    auth: &SecretFields,
) -> Result<ResolvedApplication> {
    if app.version_endpoints.is_empty() {
        bail!(
            "Application {} has no Version Endpoints",
            app.application.name
        );
    }
    let mut failures = Vec::new();
    for endpoint in &app.version_endpoints {
        match execute_version_endpoint(target, app, endpoint, auth) {
            Ok(value) => {
                let Some(version) = value.pointer(&endpoint.pointer).and_then(|v| v.as_str())
                else {
                    failures.push(format!(
                        "{} {} did not return a string at {}",
                        endpoint.method, endpoint.path, endpoint.pointer
                    ));
                    continue;
                };
                if Version::parse(version).is_err() {
                    failures.push(format!(
                        "{} {} returned invalid Application Version",
                        endpoint.method, endpoint.path
                    ));
                    continue;
                }
                return resolve_version(app, target, version);
            }
            Err(error) => failures.push(format!(
                "{} {} failed: {error}",
                endpoint.method, endpoint.path
            )),
        }
    }
    bail!(
        "Application Version discovery exhausted all Version Endpoints: {}",
        failures.join("; ")
    )
}

pub(crate) fn resolve_version(
    app: &ApplicationDefinition,
    target: &TargetConfig,
    version_text: &str,
) -> Result<ResolvedApplication> {
    let version = Version::parse(version_text)
        .with_context(|| format!("invalid Application Version {version_text}"))?;
    let catalog = app.catalogs.get(&version.major).with_context(|| {
        format!(
            "Application {} does not support major version {}",
            app.application.name, version.major
        )
    })?;
    let application_requirement = requirement(&catalog.application.version, "Application")?;
    if !application_requirement.matches(&version) {
        bail!(
            "Application Version {version} does not match catalog constraint {}",
            catalog.application.version
        );
    }
    resolve_catalog(catalog, target, &version)
}

fn resolve_catalog(
    catalog: &ResourceTypeCatalog,
    target: &TargetConfig,
    version: &Version,
) -> Result<ResolvedApplication> {
    let mut selected = BTreeMap::new();
    let mut resource_types = BTreeMap::new();
    for (name, definitions) in &catalog.resource_types {
        let mut matches = Vec::new();
        for definition in definitions {
            let constraint = definition
                .version
                .as_deref()
                .unwrap_or(&catalog.application.version);
            if requirement(constraint, &format!("Resource Type {name}"))?.matches(version) {
                matches.push((constraint, definition));
            }
        }
        if matches.len() > 1 {
            bail!(
                "Resource Type {name} has {} definitions matching Application Version {version}",
                matches.len()
            );
        }
        let Some((constraint, definition)) = matches.pop() else {
            continue;
        };
        let mut definition = definition.clone();
        definition.version = Some(constraint.to_owned());
        add_target_sensitive_fields(name, target, &mut definition)?;
        selected.insert(name.clone(), constraint.to_owned());
        resource_types.insert(name.clone(), definition);
    }
    for (name, resource_type) in &resource_types {
        for dependency in &resource_type.dependencies {
            if !resource_types.contains_key(dependency) {
                bail!(
                    "Resource Type {name} depends on unavailable Resource Type {dependency} for Application Version {version}"
                );
            }
        }
    }
    validate_dependency_cycles(&resource_types)?;
    Ok(ResolvedApplication {
        application_version: version.to_string(),
        catalog_version: catalog.version.clone(),
        selected,
        resource_types,
    })
}

fn validate_dependency_cycles(resource_types: &BTreeMap<String, ResourceType>) -> Result<()> {
    fn visit(
        name: &str,
        resource_types: &BTreeMap<String, ResourceType>,
        visiting: &mut BTreeSet<String>,
        done: &mut BTreeSet<String>,
    ) -> Result<()> {
        if done.contains(name) {
            return Ok(());
        }
        if !visiting.insert(name.into()) {
            bail!("Resource Type dependency cycle includes {name}");
        }
        for dependency in &resource_types[name].dependencies {
            visit(dependency, resource_types, visiting, done)?;
        }
        visiting.remove(name);
        done.insert(name.into());
        Ok(())
    }

    let mut visiting = BTreeSet::new();
    let mut done = BTreeSet::new();
    for name in resource_types.keys() {
        visit(name, resource_types, &mut visiting, &mut done)?;
    }
    Ok(())
}

fn requirement(value: &str, owner: &str) -> Result<VersionReq> {
    VersionReq::parse(value)
        .with_context(|| format!("{owner} has invalid version constraint {value}"))
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
    crate::hints::validate_project_placement(&root, &project)?;
    let mut applications = Vec::new();
    for entry in fs::read_dir(root.join(".taku/applications"))
        .unwrap_or_else(|_| fs::read_dir(root.join(".taku")).unwrap())
    {
        let entry = entry?;
        if entry.path().join("application.yml").is_file() {
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
            let application = load_installed(&root, &target_config.application)?;
            let baseline = load_baseline(&baseline_path(&root, environment, target)).ok();
            let resolved = for_local_use(&application, target_config, baseline.as_ref())?;
            crate::hints::validate_target_tree(
                &root,
                &project,
                environment,
                target,
                &resolved.resource_types,
            )?;
        }
    }
    Ok(serde_json::json!({
        "valid": true,
        "applications": applications,
        "environments": project.environments.len()
    }))
}

pub fn baseline_from(resolved: &ResolvedApplication) -> TargetBaseline {
    TargetBaseline {
        schema_version: SCHEMA_VERSION,
        application_version: resolved.application_version.clone(),
        catalog_version: resolved.catalog_version.clone(),
        definitions: resolved.selected.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_catalog;
    use crate::{ApiStability, CatalogApplicationIdentity, ResourceTypeCatalog, TargetConfig};

    #[test]
    fn omitted_resource_version_inherits_application_version_and_stability_defaults_stable() {
        let yaml = r#"
schema_version: 1
version: "2.0.0"
application: { name: example, version: ">=9.0.0, <10.0.0" }
resource_types:
  things:
    - id: { pointer: /id, scope: universal }
      display_name: { pointer: /name, strategy: name }
      operations:
        read: { method: GET, path: "/things/{id}", cardinality: one }
        create: { method: POST, path: /things, cardinality: one }
        update: { method: PUT, path: "/things/{id}", cardinality: one }
"#;
        let catalog: ResourceTypeCatalog = serde_yaml::from_str(yaml).unwrap();
        let target: TargetConfig =
            serde_yaml::from_str("application: example\nurl: http://example.invalid\n").unwrap();
        let resolved =
            resolve_catalog(&catalog, &target, &semver::Version::parse("9.4.0").unwrap()).unwrap();
        let definition = &resolved.resource_types["things"];
        assert_eq!(definition.version.as_deref(), Some(">=9.0.0, <10.0.0"));
        assert_eq!(definition.stability, ApiStability::Stable);
    }

    #[test]
    fn zero_matches_skips_and_multiple_matches_fail() {
        let mut catalog = ResourceTypeCatalog {
            schema_version: 1,
            version: "1.0.0".into(),
            application: CatalogApplicationIdentity {
                name: "example".into(),
                version: ">=9.0.0, <10.0.0".into(),
            },
            resource_types: Default::default(),
        };
        let definition: crate::ResourceType = serde_yaml::from_str(
            r#"
version: ">=9.5.0"
id: { pointer: /id, scope: universal }
display_name: { pointer: /name, strategy: name }
operations:
  read: { method: GET, path: "/things/{id}", cardinality: one }
  create: { method: POST, path: /things, cardinality: one }
  update: { method: PUT, path: "/things/{id}", cardinality: one }
"#,
        )
        .unwrap();
        catalog
            .resource_types
            .insert("things".into(), vec![definition.clone()]);
        let target: TargetConfig =
            serde_yaml::from_str("application: example\nurl: http://example.invalid\n").unwrap();
        let resolved =
            resolve_catalog(&catalog, &target, &semver::Version::parse("9.4.0").unwrap()).unwrap();
        assert!(resolved.resource_types.is_empty());

        catalog
            .resource_types
            .get_mut("things")
            .unwrap()
            .push(definition);
        assert!(
            resolve_catalog(&catalog, &target, &semver::Version::parse("9.5.0").unwrap()).is_err()
        );
    }
}
