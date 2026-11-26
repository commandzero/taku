use crate::canonical::{InventoryEntry, Selection, list_inventory};
use crate::lifecycle::{DeletionMarker, deletion_markers};
use crate::{GitPolicy, RepositoryLayout, git_root, load_project};
use anyhow::{Result, bail};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn push_confirmation_required(
    root: &Path,
    selection: &Selection,
    uncommitted: Option<GitPolicy>,
    untracked: Option<GitPolicy>,
) -> Result<bool> {
    let root = git_root(root)?;
    let project = load_project(&root)?;
    let inventory = list_inventory(&root, selection)?;
    let markers = deletion_markers(&root, selection)?;
    let marker_paths = marker_paths(&root, &markers);
    let (has_uncommitted, has_untracked) = selected_git_changes(&root, &inventory, &marker_paths)?;
    let uncommitted_policy = uncommitted
        .or(project.push.uncommitted)
        .unwrap_or(GitPolicy::Confirm);
    let untracked_policy = untracked
        .or(project.push.untracked)
        .unwrap_or(GitPolicy::Block);
    let uncommitted_confirmation =
        confirmation_for_policy("uncommitted", has_uncommitted, uncommitted_policy)?;
    let untracked_confirmation =
        confirmation_for_policy("untracked", has_untracked, untracked_policy)?;
    Ok(uncommitted_confirmation || untracked_confirmation)
}

fn confirmation_for_policy(kind: &str, present: bool, policy: GitPolicy) -> Result<bool> {
    if !present || policy == GitPolicy::Allow {
        return Ok(false);
    }
    match policy {
        GitPolicy::Confirm => Ok(true),
        GitPolicy::Block => {
            bail!("selected {kind} Resources are blocked by Push Git-State Policy")
        }
        GitPolicy::Allow => Ok(false),
    }
}

pub(super) fn git_revision(root: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(root)
        .output()?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().into())
    } else {
        Ok("unborn".into())
    }
}
pub(super) struct GitPolicyCheck {
    pub(super) uncommitted: Option<GitPolicy>,
    pub(super) untracked: Option<GitPolicy>,
    pub(super) interactive: bool,
    pub(super) confirmed: bool,
}

pub(super) fn enforce_git(
    root: &Path,
    items: &[InventoryEntry],
    marker_paths: &[String],
    policy: GitPolicyCheck,
) -> Result<()> {
    let (has_uncommitted, has_untracked) = selected_git_changes(root, items, marker_paths)?;
    let default_uncommitted = if policy.interactive {
        GitPolicy::Confirm
    } else {
        GitPolicy::Block
    };
    policy.check(
        "uncommitted",
        has_uncommitted,
        policy.uncommitted.unwrap_or(default_uncommitted),
    )?;
    policy.check(
        "untracked",
        has_untracked,
        policy.untracked.unwrap_or(GitPolicy::Block),
    )
}

fn selected_git_changes(
    root: &Path,
    items: &[InventoryEntry],
    marker_paths: &[String],
) -> Result<(bool, bool)> {
    let project = load_project(root)?;
    let mut paths = BTreeSet::new();
    for item in items {
        add_resource_path(&mut paths, root, &item.path)?;
        let target = crate::canonical::target_root(root, &project, &item.environment, &item.target);
        add_hint(&mut paths, root, &target, crate::hints::TARGET_HINT_NAME)?;
    }
    for marker in marker_paths {
        add_resource_path(&mut paths, root, marker)?;
        if let Some(target) = marker_target_dir(root, marker, project.layout) {
            add_hint(&mut paths, root, &target, crate::hints::TARGET_HINT_NAME)?;
        }
    }
    if paths.is_empty() {
        return Ok((false, false));
    }
    let mut command = Command::new("git");
    command.args(["status", "--porcelain", "--"]);
    command.args(paths);
    let output = command.current_dir(root).output()?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok((
        text.lines().any(|line| !line.starts_with("??")),
        text.lines().any(|line| line.starts_with("??")),
    ))
}
fn add_resource_path(paths: &mut BTreeSet<String>, root: &Path, path: &str) -> Result<()> {
    paths.insert(path.to_owned());
    if let Some(parent) = root.join(path).parent() {
        add_hint(paths, root, parent, crate::hints::RESOURCE_HINT_NAME)?;
    }
    Ok(())
}

fn add_hint(paths: &mut BTreeSet<String>, root: &Path, directory: &Path, hint: &str) -> Result<()> {
    paths.insert(
        directory
            .join(hint)
            .strip_prefix(root)?
            .display()
            .to_string(),
    );
    Ok(())
}

fn marker_target_dir(root: &Path, path: &str, layout: RepositoryLayout) -> Option<PathBuf> {
    let mut components = Path::new(path).components();
    let first = components.next()?;
    let directory = root.join(first.as_os_str());
    match layout {
        RepositoryLayout::Single => Some(directory),
        RepositoryLayout::Multi => Some(directory.join(components.next()?.as_os_str())),
    }
}

impl GitPolicyCheck {
    fn check(&self, kind: &str, present: bool, policy: GitPolicy) -> Result<()> {
        if !present {
            return Ok(());
        }
        match policy {
            GitPolicy::Allow => Ok(()),
            GitPolicy::Confirm if self.interactive && self.confirmed => Ok(()),
            GitPolicy::Confirm if self.interactive => {
                bail!("selected {kind} Resources require confirmation; repeat with --yes")
            }
            GitPolicy::Confirm | GitPolicy::Block => {
                bail!("selected {kind} Resources are blocked by Push Git-State Policy")
            }
        }
    }
}

pub(super) fn marker_paths(root: &Path, markers: &[(PathBuf, DeletionMarker)]) -> Vec<String> {
    markers
        .iter()
        .map(|(path, _)| path.strip_prefix(root).unwrap().display().to_string())
        .collect()
}
