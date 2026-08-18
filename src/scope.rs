use anyhow::{Result, bail};
use resource_control::{Selection, load_project};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourcePath {
    Target {
        target: String,
    },
    Type {
        target: String,
        resource_type: String,
    },
    Resources {
        target: String,
        resource_type: String,
        ids: Vec<String>,
    },
}

impl ResourcePath {
    pub fn partial(
        target: Option<String>,
        resource_type: Option<String>,
        ids: Vec<String>,
    ) -> Result<Option<Self>> {
        match (target, resource_type, ids.is_empty()) {
            (None, None, true) => Ok(None),
            (Some(target), None, true) => Ok(Some(Self::Target { target })),
            (Some(target), Some(resource_type), true) => Ok(Some(Self::Type {
                target,
                resource_type,
            })),
            (Some(target), Some(resource_type), false) => Ok(Some(Self::Resources {
                target,
                resource_type,
                ids,
            })),
            (None, Some(_), _) => bail!("Resource Type requires a Target"),
            (_, None, false) => bail!("Resource IDs require a Target and Resource Type"),
        }
    }

    pub fn exact(target: String, resource_type: String, ids: Vec<String>) -> Result<Self> {
        if ids.is_empty() {
            bail!("at least one Resource ID is required");
        }
        Ok(Self::Resources {
            target,
            resource_type,
            ids,
        })
    }

    pub fn target(&self) -> &str {
        match self {
            Self::Target { target }
            | Self::Type { target, .. }
            | Self::Resources { target, .. } => target,
        }
    }

    pub fn resource_type(&self) -> Option<&str> {
        match self {
            Self::Target { .. } => None,
            Self::Type { resource_type, .. } | Self::Resources { resource_type, .. } => {
                Some(resource_type)
            }
        }
    }

    pub fn ids(&self) -> &[String] {
        match self {
            Self::Resources { ids, .. } => ids,
            Self::Target { .. } | Self::Type { .. } => &[],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopePolicy {
    Exact,
    Partial,
    RemoteList,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvironmentSelection {
    pub names: Vec<String>,
    pub all: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceScope {
    pub environments: EnvironmentSelection,
    pub path: Option<ResourcePath>,
    pub namespace: Option<String>,
}

impl ResourceScope {
    pub fn new(
        environments: EnvironmentSelection,
        path: Option<ResourcePath>,
        namespace: Option<String>,
    ) -> Self {
        Self {
            environments,
            path,
            namespace,
        }
    }

    pub fn selections(&self, root: &Path, policy: ScopePolicy) -> Result<Vec<Selection>> {
        self.validate(policy)?;
        let environments = if self.environments.all {
            load_project(root)?
                .environments
                .keys()
                .cloned()
                .map(Some)
                .collect()
        } else if self.environments.names.is_empty() {
            vec![None]
        } else {
            self.environments.names.iter().cloned().map(Some).collect()
        };
        let (targets, types, ids) = match &self.path {
            None => (vec![], vec![], vec![]),
            Some(path) => (
                vec![path.target().to_owned()],
                path.resource_type()
                    .map(str::to_owned)
                    .into_iter()
                    .collect(),
                path.ids().to_vec(),
            ),
        };
        let namespaces: Vec<String> = self.namespace.clone().into_iter().collect();
        Ok(environments
            .into_iter()
            .map(|environment| Selection {
                environment,
                targets: targets.clone(),
                namespaces: namespaces.clone(),
                types: types.clone(),
                ids: ids.clone(),
            })
            .collect())
    }

    #[cfg(test)]
    pub fn validate_namespace_kind(
        &self,
        namespaced: bool,
        exact_namespace_required: bool,
    ) -> Result<()> {
        if namespaced && exact_namespace_required && self.namespace.is_none() {
            bail!("--namespace is required for this namespaced Resource Type");
        }
        if !namespaced && self.namespace.is_some() {
            bail!("--namespace is not valid for a non-namespaced Resource Type");
        }
        Ok(())
    }

    fn validate(&self, policy: ScopePolicy) -> Result<()> {
        match policy {
            ScopePolicy::Exact => {
                if !matches!(self.path, Some(ResourcePath::Resources { .. })) {
                    bail!("Target, Resource Type, and at least one Resource ID are required");
                }
            }
            ScopePolicy::Partial => {}
            ScopePolicy::RemoteList => {
                if !matches!(
                    self.path,
                    Some(ResourcePath::Type { .. } | ResourcePath::Resources { .. })
                ) {
                    bail!("remote List requires a Target and Resource Type");
                }
            }
        }
        let exact_environment =
            self.path.is_some() || matches!(policy, ScopePolicy::Exact | ScopePolicy::RemoteList);
        if exact_environment && (self.environments.all || self.environments.names.len() > 1) {
            bail!("a Resource Path accepts exactly one Environment");
        }
        if self.namespace.is_some()
            && !matches!(
                self.path,
                Some(ResourcePath::Type { .. } | ResourcePath::Resources { .. })
            )
        {
            bail!("--namespace requires a Target and Resource Type");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environments(names: &[&str], all: bool) -> EnvironmentSelection {
        EnvironmentSelection {
            names: names.iter().map(|name| (*name).to_owned()).collect(),
            all,
        }
    }

    #[test]
    fn partial_path_supports_every_contiguous_prefix() {
        assert_eq!(ResourcePath::partial(None, None, vec![]).unwrap(), None);
        assert_eq!(
            ResourcePath::partial(Some("es".into()), None, vec![]).unwrap(),
            Some(ResourcePath::Target {
                target: "es".into()
            })
        );
        assert_eq!(
            ResourcePath::partial(Some("es".into()), Some("roles".into()), vec![]).unwrap(),
            Some(ResourcePath::Type {
                target: "es".into(),
                resource_type: "roles".into()
            })
        );
        assert_eq!(
            ResourcePath::partial(
                Some("es".into()),
                Some("roles".into()),
                vec!["one".into(), "two".into()]
            )
            .unwrap(),
            Some(ResourcePath::Resources {
                target: "es".into(),
                resource_type: "roles".into(),
                ids: vec!["one".into(), "two".into()]
            })
        );
    }

    #[test]
    fn partial_path_rejects_hierarchy_gaps() {
        assert!(ResourcePath::partial(None, Some("roles".into()), vec![]).is_err());
        assert!(ResourcePath::partial(Some("es".into()), None, vec!["one".into()]).is_err());
    }

    #[test]
    fn exact_path_requires_ids() {
        assert!(ResourcePath::exact("es".into(), "roles".into(), vec![]).is_err());
        assert!(ResourcePath::exact("es".into(), "roles".into(), vec!["one".into()]).is_ok());
    }

    #[test]
    fn exact_and_remote_policies_require_their_path_shapes() {
        let target = ResourceScope::new(
            environments(&[], false),
            Some(ResourcePath::Target {
                target: "es".into(),
            }),
            None,
        );
        assert!(
            target
                .selections(Path::new("."), ScopePolicy::Exact)
                .is_err()
        );
        assert!(
            target
                .selections(Path::new("."), ScopePolicy::RemoteList)
                .is_err()
        );
        assert!(
            target
                .selections(Path::new("."), ScopePolicy::Partial)
                .is_ok()
        );
    }

    #[test]
    fn a_path_rejects_multiple_or_all_environments() {
        let path = Some(ResourcePath::Type {
            target: "es".into(),
            resource_type: "roles".into(),
        });
        let multiple =
            ResourceScope::new(environments(&["dev", "prod"], false), path.clone(), None);
        let all = ResourceScope::new(environments(&[], true), path, None);
        assert!(
            multiple
                .selections(Path::new("."), ScopePolicy::Partial)
                .is_err()
        );
        assert!(
            all.selections(Path::new("."), ScopePolicy::Partial)
                .is_err()
        );
    }

    #[test]
    fn namespace_requires_target_and_type_and_matches_type_kind() {
        let broad = ResourceScope::new(environments(&[], false), None, Some("default".into()));
        assert!(
            broad
                .selections(Path::new("."), ScopePolicy::Partial)
                .is_err()
        );
        let typed = ResourceScope::new(
            environments(&[], false),
            Some(ResourcePath::Type {
                target: "kb".into(),
                resource_type: "saved_objects".into(),
            }),
            Some("default".into()),
        );
        assert!(typed.validate_namespace_kind(true, true).is_ok());
        assert!(typed.validate_namespace_kind(false, false).is_err());
        let missing = ResourceScope::new(
            environments(&[], false),
            Some(ResourcePath::Resources {
                target: "kb".into(),
                resource_type: "saved_objects".into(),
                ids: vec!["one".into()],
            }),
            None,
        );
        assert!(missing.validate_namespace_kind(true, true).is_err());
    }

    #[test]
    fn scope_converts_one_parent_path_to_selection_vectors() {
        let scope = ResourceScope::new(
            environments(&["dev"], false),
            Some(ResourcePath::Resources {
                target: "es".into(),
                resource_type: "roles".into(),
                ids: vec!["one".into(), "two".into()],
            }),
            None,
        );
        let selections = scope
            .selections(Path::new("."), ScopePolicy::Exact)
            .unwrap();
        assert_eq!(selections.len(), 1);
        assert_eq!(selections[0].environment.as_deref(), Some("dev"));
        assert_eq!(selections[0].targets, vec!["es"]);
        assert_eq!(selections[0].types, vec!["roles"]);
        assert_eq!(selections[0].ids, vec!["one", "two"]);
    }
}
