use crate::application::load_installed;
use crate::canonical::Selection;
use crate::project::current_environment;
use crate::provider::resolve_auth;
use crate::resolution::{
    ResolvedApplication, baseline_from, baseline_path, discover, save_baseline,
};
use crate::{git_root, load_project};
use anyhow::Result;
use std::collections::{BTreeMap, btree_map::Entry};
use std::path::{Path, PathBuf};

pub(super) struct RemoteDiscovery {
    pub(super) root: PathBuf,
    pub(super) project: crate::Project,
    pub(super) environment: String,
    pub(super) resolved: BTreeMap<String, ResolvedApplication>,
    pub(super) missing_baselines: Vec<(PathBuf, crate::TargetBaseline)>,
}

pub(super) fn discover_remote(
    root: &Path,
    selection: &Selection,
    cli_provider: &BTreeMap<String, String>,
) -> Result<RemoteDiscovery> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    crate::hints::validate_project_placement(&root, &project)?;
    let environment = current_environment(&root, &project, selection.environment.as_deref())?;
    let mut resolved = BTreeMap::new();
    let mut missing_baselines = Vec::new();
    for (target_name, target) in &project.environments[&environment].targets {
        if !selection.targets.is_empty() && !selection.targets.contains(target_name) {
            continue;
        }
        let app = load_installed(&root, &target.application)?;
        crate::hints::validate_application_tree(&root, &project, &environment, target_name, &app)?;
        let auth = resolve_auth(&root, &project, &environment, target, cli_provider)?;
        let discovered = discover(&app, target, &auth)?;
        let baseline = baseline_path(&root, &environment, target_name);
        if !baseline.exists() {
            missing_baselines.push((baseline, baseline_from(&discovered)));
        }
        resolved.insert(target_name.clone(), discovered);
    }
    Ok(RemoteDiscovery {
        root,
        project,
        environment,
        resolved,
        missing_baselines,
    })
}

impl RemoteDiscovery {
    pub(super) fn persist_baselines(&self) -> Result<()> {
        for (path, baseline) in &self.missing_baselines {
            save_baseline(path, baseline)?;
        }
        Ok(())
    }

    pub(super) fn resolved_target<'a>(
        &'a mut self,
        name: &str,
        app: &crate::ApplicationDefinition,
        provider: &BTreeMap<String, String>,
    ) -> Result<&'a ResolvedApplication> {
        let target = &self.project.environments[&self.environment].targets[name];
        Ok(match self.resolved.entry(name.to_owned()) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let auth = resolve_auth(
                    &self.root,
                    &self.project,
                    &self.environment,
                    target,
                    provider,
                )?;
                let discovered = discover(app, target, &auth)?;
                let baseline = baseline_path(&self.root, &self.environment, name);
                if !baseline.exists() {
                    save_baseline(&baseline, &baseline_from(&discovered))?;
                }
                entry.insert(discovered)
            }
        })
    }
}
