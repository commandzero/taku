use crate::application::{list_applications, load_installed};
use crate::canonical::{Selection, list_inventory};
use crate::project::current_environment;
use crate::resolution::{baseline_path, for_local_use, load_baseline};
use crate::{git_root, load_project, remote_list_read_only, remote_resource_types_read_only};
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct CompletionCandidate {
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompletionIntent {
    Environment,
    InstallApplication,
    UpdateApplication,
    TargetApplication,
    Target,
    PromotionTarget,
    ProviderKey,
    LocalResourceType { include_markers: bool },
    LocalResourceId { include_markers: bool },
    RemoteResourceType,
    RemoteResourceId { untracked_only: bool },
    Namespace { remote: bool },
}

#[derive(Clone, Debug)]
pub struct CompletionQuery {
    pub project: PathBuf,
    pub intent: CompletionIntent,
    pub environment: Option<String>,
    pub target: Option<String>,
    pub resource_type: Option<String>,
    pub namespace: Option<String>,
    pub prefix: String,
    pub selected: Vec<String>,
    pub provider: BTreeMap<String, String>,
}

impl CompletionQuery {
    pub fn new(project: impl Into<PathBuf>, intent: CompletionIntent) -> Self {
        Self {
            project: project.into(),
            intent,
            environment: None,
            target: None,
            resource_type: None,
            namespace: None,
            prefix: String::new(),
            selected: Vec::new(),
            provider: BTreeMap::new(),
        }
    }
}

pub fn completion_candidates(query: &CompletionQuery) -> Result<Vec<CompletionCandidate>> {
    let root = git_root(&query.project)?;
    let project = load_project(&root)?;
    let mut candidates = match &query.intent {
        CompletionIntent::Environment => project
            .environments
            .keys()
            .map(|name| candidate(name, "Environment"))
            .collect(),
        CompletionIntent::InstallApplication => list_applications(&root)?
            .into_iter()
            .filter(|application| !application.installed)
            .map(|application| {
                candidate(
                    &application.name,
                    &format!("Application {}", application.version),
                )
            })
            .collect(),
        CompletionIntent::UpdateApplication => list_applications(&root)?
            .into_iter()
            .filter(|application| application.installed)
            .map(|application| {
                candidate(
                    &application.name,
                    &format!("installed Application {}", application.version),
                )
            })
            .collect(),
        CompletionIntent::TargetApplication => list_applications(&root)?
            .into_iter()
            .map(|application| {
                candidate(
                    &application.name,
                    &format!("Application {}", application.version),
                )
            })
            .collect(),
        CompletionIntent::Target | CompletionIntent::PromotionTarget => {
            let environment = current_environment(&root, &project, query.environment.as_deref())?;
            project.environments[&environment]
                .targets
                .iter()
                .map(|(name, target)| candidate(name, &target.application))
                .collect()
        }
        CompletionIntent::ProviderKey => provider_candidates(&root, &project, query)?,
        CompletionIntent::LocalResourceType { include_markers } => {
            local_type_candidates(&root, query, *include_markers)?
        }
        CompletionIntent::RemoteResourceType => remote_resource_types_read_only(
            &root,
            query.environment.as_deref(),
            query.target.as_deref().context("Target is required")?,
            &query.provider,
        )?
        .into_iter()
        .map(|resource_type| {
            candidate(
                &resource_type.name,
                if resource_type.namespaced {
                    "namespaced remote Resource Type"
                } else {
                    "remote Resource Type"
                },
            )
        })
        .collect(),
        CompletionIntent::LocalResourceId { include_markers } => {
            local_id_candidates(&root, &project, query, *include_markers)?
        }
        CompletionIntent::RemoteResourceId { untracked_only } => {
            let selection = exact_selection(query)?;
            remote_list_read_only(&root, &selection, *untracked_only, &query.provider)?
                .into_iter()
                .map(|entry| candidate(&entry.id, &entry.name))
                .collect()
        }
        CompletionIntent::Namespace { remote } => {
            namespace_candidates(&root, &project, query, *remote)?
        }
    };

    let selected: BTreeSet<_> = query.selected.iter().collect();
    candidates.retain(|candidate| {
        candidate.value.starts_with(&query.prefix) && !selected.contains(&candidate.value)
    });
    candidates.sort();
    candidates.dedup_by(|left, right| left.value == right.value);
    Ok(candidates)
}

fn candidate(value: &str, description: &str) -> CompletionCandidate {
    CompletionCandidate {
        value: value.to_owned(),
        description: Some(description.to_owned()),
    }
}

fn exact_selection(query: &CompletionQuery) -> Result<Selection> {
    Ok(Selection {
        environment: query.environment.clone(),
        targets: vec![query.target.clone().context("Target is required")?],
        namespaces: query.namespace.clone().into_iter().collect(),
        types: vec![
            query
                .resource_type
                .clone()
                .context("Resource Type is required")?,
        ],
        ids: Vec::new(),
    })
}

fn local_id_candidates(
    root: &std::path::Path,
    project: &crate::Project,
    query: &CompletionQuery,
    include_markers: bool,
) -> Result<Vec<CompletionCandidate>> {
    if local_resource_type_namespaced(root, project, query)? && query.namespace.is_none() {
        return Ok(Vec::new());
    }
    let selection = exact_selection(query)?;
    let mut candidates: Vec<_> = list_inventory(root, &selection)?
        .into_iter()
        .map(|entry| candidate(&entry.id, &entry.display_name))
        .collect();
    if include_markers {
        for (_, marker) in crate::lifecycle::deletion_markers(root, &selection)? {
            candidates.push(candidate(&marker.id, "Deletion Marker"));
        }
    }
    Ok(candidates)
}

fn local_type_candidates(
    root: &std::path::Path,
    query: &CompletionQuery,
    include_markers: bool,
) -> Result<Vec<CompletionCandidate>> {
    let selection = Selection {
        environment: query.environment.clone(),
        targets: vec![query.target.clone().context("Target is required")?],
        namespaces: query.namespace.clone().into_iter().collect(),
        types: Vec::new(),
        ids: Vec::new(),
    };
    let mut values: BTreeSet<String> = list_inventory(root, &selection)?
        .into_iter()
        .map(|entry| entry.resource_type)
        .collect();
    if include_markers {
        values.extend(
            crate::lifecycle::deletion_markers(root, &selection)?
                .into_iter()
                .map(|(_, marker)| marker.resource_type),
        );
    }
    Ok(values
        .into_iter()
        .map(|value| candidate(&value, "managed Resource Type"))
        .collect())
}

fn namespace_candidates(
    root: &std::path::Path,
    project: &crate::Project,
    query: &CompletionQuery,
    remote: bool,
) -> Result<Vec<CompletionCandidate>> {
    let namespaced = if remote {
        let target = query.target.as_deref().context("Target is required")?;
        let resource_type = query
            .resource_type
            .as_deref()
            .context("Resource Type is required")?;
        remote_resource_types_read_only(
            root,
            query.environment.as_deref(),
            target,
            &query.provider,
        )?
        .into_iter()
        .find(|candidate| candidate.name == resource_type)
        .with_context(|| format!("unknown or non-listable Resource Type {resource_type}"))?
        .namespaced
    } else {
        local_resource_type_namespaced(root, project, query)?
    };
    if !namespaced {
        return Ok(Vec::new());
    }
    let mut values = BTreeSet::from(["default".to_owned()]);
    let mut selection = exact_selection(query)?;
    selection.namespaces.clear();
    for entry in list_inventory(root, &selection)? {
        if let Some(namespace) = entry.namespace {
            values.insert(namespace);
        }
    }
    for (_, marker) in crate::lifecycle::deletion_markers(root, &selection)? {
        if let Some(namespace) = marker.namespace {
            values.insert(namespace);
        }
    }
    Ok(values
        .into_iter()
        .map(|value| candidate(&value, "Namespace"))
        .collect())
}

fn local_resource_type_namespaced(
    root: &std::path::Path,
    project: &crate::Project,
    query: &CompletionQuery,
) -> Result<bool> {
    let environment = current_environment(root, project, query.environment.as_deref())?;
    let target_name = query.target.as_deref().context("Target is required")?;
    let target = project.environments[&environment]
        .targets
        .get(target_name)
        .with_context(|| format!("unknown Target {target_name}"))?;
    let application = load_installed(root, &target.application)?;
    let baseline = load_baseline(&baseline_path(root, &environment, target_name)).ok();
    let resolved = for_local_use(&application, target, baseline.as_ref())?;
    let resource_type = query
        .resource_type
        .as_deref()
        .context("Resource Type is required")?;
    Ok(resolved
        .resource_types
        .get(resource_type)
        .with_context(|| format!("unknown Resource Type {resource_type}"))?
        .namespaced)
}

fn provider_candidates(
    root: &std::path::Path,
    project: &crate::Project,
    query: &CompletionQuery,
) -> Result<Vec<CompletionCandidate>> {
    let environment = current_environment(root, project, query.environment.as_deref())?;
    let env = &project.environments[&environment];
    let mut fields = BTreeSet::new();
    if let Some(provider) = &env.provider {
        fields.extend(provider.fields.keys().cloned());
    }
    if let Some(target_name) = query.target.as_deref()
        && let Some(auth) = env
            .targets
            .get(target_name)
            .and_then(|target| target.auth.as_ref())
    {
        fields.extend(auth.fields.keys().cloned());
    }
    Ok(fields
        .into_iter()
        .map(|field| candidate(&format!("{field}="), "provider field"))
        .collect())
}
