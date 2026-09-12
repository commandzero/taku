//! Dependency-free repository gate, compiled by check-openspec.sh.
#![forbid(unsafe_code)]
use std::collections::{BTreeMap, BTreeSet};
use std::{env, fs, process::Command};

type Files = BTreeMap<String, String>;

fn git(args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}

fn git_show_optional(revision: &str, path: &str) -> Result<Option<String>, String> {
    let object = format!("{revision}:{path}");
    let probe = Command::new("git")
        .args(["cat-file", "-e", &object])
        .output()
        .map_err(|e| e.to_string())?;
    if probe.status.success() {
        return git(&["show", &object]).map(Some);
    }
    let stderr = String::from_utf8_lossy(&probe.stderr);
    if stderr.starts_with("fatal: path '")
        && (stderr.contains("' does not exist in '")
            || stderr.contains("' exists on disk, but not in '"))
    {
        Ok(None)
    } else {
        Err(stderr.into_owned())
    }
}

fn change_id(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("openspec/changes/")?;
    if let Some(rest) = rest.strip_prefix("archive/") {
        let archive = rest.split('/').next()?;
        let (date, id) = archive.split_at_checked(11)?;
        if date.bytes().enumerate().all(|(i, c)| {
            if [4, 7, 10].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_digit()
            }
        }) && !id.is_empty()
            && id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        {
            Some(id)
        } else {
            None
        }
    } else {
        rest.split_once('/').map(|(id, _)| id)
    }
}

fn touched_ids(paths: &[String]) -> Result<BTreeSet<String>, String> {
    for path in paths {
        if let Some(rest) = path.strip_prefix("openspec/changes/archive/") {
            let valid = rest.split_once('/').is_some_and(|(archive, file)| {
                !archive.is_empty() && !file.is_empty() && change_id(path).is_some()
            });
            if !valid {
                return Err(format!(
                    "Invalid archive path, expected YYYY-MM-DD-change-id/file: {path}"
                ));
            }
        }
    }
    Ok(paths
        .iter()
        .filter_map(|p| change_id(p).map(str::to_owned))
        .collect())
}

fn select(paths: &[String], explicit: &str) -> Result<BTreeSet<String>, String> {
    let mut ids = touched_ids(paths)?;
    for id in parse_explicit_ids(explicit)? {
        ids.insert(id);
    }
    Ok(ids)
}

fn parse_explicit_ids(explicit: &str) -> Result<BTreeSet<String>, String> {
    let trimmed = explicit.trim();
    if trimmed.is_empty() || trimmed == "none" {
        return Ok(BTreeSet::new());
    }
    let mut ids = BTreeSet::new();
    let fields: Vec<&str> = if trimmed.contains(',') {
        trimmed.split(',').map(str::trim).collect()
    } else {
        trimmed.split_whitespace().collect()
    };
    for id in fields {
        if id.is_empty() || id == "none" {
            return Err(format!("Invalid OpenSpec change ID list: {explicit}"));
        }
        if !id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        {
            return Err(format!("Invalid OpenSpec change ID: {id}"));
        }
        ids.insert(id.to_owned());
    }
    if ids.is_empty() {
        return Err(format!("Invalid OpenSpec change ID list: {explicit}"));
    }
    Ok(ids)
}

fn parse_association(value: &str) -> Result<BTreeSet<String>, String> {
    let trimmed = value.trim();
    if trimmed == "none" {
        return Ok(BTreeSet::new());
    }
    if trimmed.is_empty() {
        return Err("OpenSpec association must be none or one or more change IDs".into());
    }
    parse_explicit_ids(trimmed)
}

fn require_association_for_main_specs(
    paths: &[String],
    ids: &BTreeSet<String>,
) -> Result<(), String> {
    let touched_main_specs = paths
        .iter()
        .any(|path| path.starts_with("openspec/specs/") && path.ends_with(".md"));
    if touched_main_specs && ids.is_empty() {
        return Err(
            "main OpenSpec specs changed without an associated archived change; set OpenSpec: change-id"
                .into(),
        );
    }
    Ok(())
}

// Compare complete requirement/scenario text, tolerating only whitespace changes.
fn requirements(text: &str) -> Result<BTreeMap<String, (String, String)>, String> {
    let mut result = BTreeMap::new();
    let mut section = String::new();
    let mut current: Option<(String, String, String)> = None;
    let save = |current: &mut Option<(String, String, String)>,
                result: &mut BTreeMap<String, (String, String)>|
     -> Result<(), String> {
        if let Some((name, mode, body)) = current.take() {
            if result
                .insert(
                    name.clone(),
                    (mode, body.split_whitespace().collect::<Vec<_>>().join(" ")),
                )
                .is_some()
            {
                return Err(format!("Duplicate requirement: {name}"));
            }
        }
        Ok(())
    };
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("## ") {
            save(&mut current, &mut result)?;
            section = value.to_owned();
        } else if let Some(name) = line.strip_prefix("### Requirement: ") {
            save(&mut current, &mut result)?;
            current = Some((name.trim().to_owned(), section.clone(), String::new()));
        } else if let Some((_, _, body)) = current.as_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    save(&mut current, &mut result)?;
    Ok(result)
}

fn non_requirement_text(text: &str) -> String {
    let mut in_requirement = false;
    let mut result = String::new();
    for line in text.lines() {
        if line.starts_with("## ") {
            in_requirement = false;
            result.push_str(line);
            result.push('\n');
        } else if line.starts_with("### Requirement: ") {
            in_requirement = true;
        } else if !in_requirement {
            result.push_str(line);
            result.push('\n');
        }
    }
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn delta_names(delta: &str) -> Result<BTreeSet<String>, String> {
    let mut names: BTreeSet<String> = requirements(delta)?.into_keys().collect();
    let mut renamed = false;
    for line in delta.lines() {
        if line.starts_with("## ") {
            renamed = line == "## RENAMED Requirements";
        } else if renamed {
            if let Some(name) = line
                .strip_prefix("- FROM: `### Requirement: ")
                .and_then(|s| s.strip_suffix('`'))
            {
                names.insert(name.to_owned());
            } else if let Some(name) = line
                .strip_prefix("- TO: `### Requirement: ")
                .and_then(|s| s.strip_suffix('`'))
            {
                names.insert(name.to_owned());
            }
        }
    }
    Ok(names)
}

fn check(files: &Files, ids: &BTreeSet<String>) -> Result<(), String> {
    for id in ids {
        let active = format!("openspec/changes/{id}/");
        if files.keys().any(|p| p.starts_with(&active)) {
            return Err(format!("{id}: synchronize and archive the active change"));
        }
        let roots: BTreeSet<String> = files
            .keys()
            .filter(|p| p.starts_with("openspec/changes/archive/") && change_id(p) == Some(id))
            .map(|p| p.split('/').take(4).collect::<Vec<_>>().join("/"))
            .collect();
        if roots.len() != 1 {
            return Err(format!(
                "{id}: expected one preserved archive, found {}",
                roots.len()
            ));
        }
        let root = roots.first().unwrap();
        for artifact in ["proposal.md", "design.md", "tasks.md"] {
            if !files
                .get(&format!("{root}/{artifact}"))
                .is_some_and(|s| !s.trim().is_empty())
            {
                return Err(format!("{id}: missing archive artifact {artifact}"));
            }
        }
        if files[&format!("{root}/tasks.md")]
            .lines()
            .any(|l| l.trim_start().starts_with("- [ ]"))
        {
            return Err(format!("{id}: unfinished archived tasks"));
        }
        let prefix = format!("{root}/specs/");
        let deltas: Vec<_> = files
            .iter()
            .filter(|(p, _)| p.starts_with(&prefix) && p.ends_with("/spec.md"))
            .collect();
        if deltas.is_empty()
            && !files
                .get(&format!("{root}/no-spec-deltas.md"))
                .is_some_and(|s| !s.trim().is_empty())
        {
            return Err(format!(
                "{id}: missing spec deltas or reviewed no-spec-deltas.md explanation"
            ));
        }
        for (path, delta) in deltas {
            let main_path = format!("openspec/specs/{}", path.strip_prefix(&prefix).unwrap());
            let main_text = files
                .get(&main_path)
                .ok_or_else(|| format!("{id}: missing main spec {main_path}"))?;
            let main = requirements(main_text)?;
            let changes = requirements(delta)?;
            if changes.is_empty() {
                return Err(format!("{id}: {path} has no requirement deltas"));
            }
            for (name, (mode, body)) in &changes {
                match mode.as_str() {
                    "ADDED Requirements" | "MODIFIED Requirements" => {
                        if !body.contains("#### Scenario:")
                            || !main.get(name).is_some_and(|(_, actual)| actual == body)
                        {
                            return Err(format!(
                                "{id}: {main_path}: unsynchronized requirement/scenarios: {name}"
                            ));
                        }
                    }
                    "REMOVED Requirements" => {
                        if main.contains_key(name) {
                            return Err(format!(
                                "{id}: {main_path}: removed requirement still present: {name}"
                            ));
                        }
                    }
                    _ => return Err(format!("{id}: unsupported delta section {mode}")),
                }
            }
            let mut renamed = false;
            let mut rename_pairs = 0;
            let mut rename_targets = BTreeSet::new();
            let mut old: Option<&str> = None;
            for line in delta.lines() {
                if line.starts_with("## ") {
                    renamed = line == "## RENAMED Requirements";
                } else if renamed {
                    if let Some(name) = line
                        .strip_prefix("- FROM: `### Requirement: ")
                        .and_then(|s| s.strip_suffix('`'))
                    {
                        if old.replace(name).is_some() {
                            return Err(format!("{id}: incomplete rename"));
                        }
                    } else if let Some(name) = line
                        .strip_prefix("- TO: `### Requirement: ")
                        .and_then(|s| s.strip_suffix('`'))
                    {
                        let previous = old
                            .take()
                            .ok_or_else(|| format!("{id}: rename missing FROM"))?;
                        if main.contains_key(previous) || !main.contains_key(name) {
                            return Err(format!(
                                "{id}: unsynchronized rename {previous} -> {name}"
                            ));
                        }
                        rename_targets.insert(name.to_owned());
                        rename_pairs += 1;
                    } else if !line.trim().is_empty() {
                        return Err(format!("{id}: unsupported rename syntax: {line}"));
                    }
                }
            }
            if old.is_some() {
                return Err(format!("{id}: rename missing TO"));
            }
            if renamed && rename_pairs == 0 {
                return Err(format!("{id}: rename section must contain a FROM/TO pair"));
            }
            for name in rename_targets {
                if !changes
                    .get(&name)
                    .is_some_and(|(mode, _)| mode == "MODIFIED Requirements")
                {
                    return Err(format!(
                        "{id}: renamed requirement must include a MODIFIED Requirements body: {name}"
                    ));
                }
            }
        }
    }
    Ok(())
}

fn run() -> Result<(), String> {
    let base = env::var("BASE_REF").ok().filter(|s| !s.is_empty());
    let mut paths = Vec::new();
    let list;
    let mut merge_base = None;
    if let Some(base) = &base {
        let revision = git(&["merge-base", base, "HEAD"])?;
        let revision = revision.trim().to_owned();
        merge_base = Some(revision.clone());
        // --no-renames yields both deleted and added paths, including directory renames.
        paths.extend(
            git(&[
                "diff",
                "--no-renames",
                "--name-only",
                "-z",
                &revision,
                "HEAD",
            ])?
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        );
        list = git(&["ls-tree", "-r", "--name-only", "-z", "HEAD", "openspec"])?;
    } else {
        paths.extend(
            git(&["diff", "--no-renames", "--name-only", "-z", "HEAD"])?
                .split('\0')
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
        );
        let untracked = git(&["ls-files", "--others", "--exclude-standard", "-z"])?;
        paths.extend(
            untracked
                .split('\0')
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
        );
        list = format!("{}{}", git(&["ls-files", "-z", "openspec"])?, untracked);
    }
    let mut files = Files::new();
    for path in list.split('\0').filter(|p| p.starts_with("openspec/")) {
        let text = if !path.ends_with(".md") {
            String::new()
        } else if base.is_some() {
            git(&["show", &format!("HEAD:{path}")])?
        } else {
            match fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.to_string()),
            }
        };
        files.insert(path.to_owned(), text);
    }
    if env::var("CHECK_ALL_ARCHIVES").as_deref() == Ok("1") {
        paths.extend(
            files
                .keys()
                .filter(|p| p.starts_with("openspec/changes/archive/"))
                .cloned(),
        );
    }
    let env_explicit = env::var("OPENSPEC_CHANGES").unwrap_or_default();
    let env_ids = parse_explicit_ids(&env_explicit)?;
    let mut pr_association = None;
    if let Ok(body) = env::var("PR_BODY") {
        let associations: Vec<_> = body
            .lines()
            .filter_map(|l| l.strip_prefix("OpenSpec:"))
            .collect();
        if associations.len() != 1 {
            return Err(
                "PR body must contain one OpenSpec: none or OpenSpec: change-id line".into(),
            );
        }
        let association = associations[0].trim();
        pr_association = Some(parse_association(association)?);
    }
    let mut ids = if pr_association.is_some() {
        // Any active change path in the PR must be synchronized and archived.
        // Active changes that are present only on the base branch are not in
        // `paths` and therefore do not gate unrelated work.
        touched_ids(&paths)?
    } else {
        select(&paths, &env_explicit)?
    };
    ids.extend(env_ids.iter().cloned());
    if let Some(association) = &pr_association {
        ids.extend(association.iter().cloned());
    }
    require_association_for_main_specs(&paths, &ids)?;
    let mut associated_ids = env_ids;
    if let Some(association) = &pr_association {
        associated_ids.extend(association.iter().cloned());
    } else {
        associated_ids.extend(ids.iter().cloned());
    }
    if let Some(association) = &pr_association {
        for touched_id in paths.iter().filter_map(|path| {
            if path.starts_with("openspec/changes/archive/") {
                change_id(path)
            } else {
                None
            }
        }) {
            if !association.contains(touched_id) {
                return Err(format!(
                    "OpenSpec association omits touched change {touched_id}"
                ));
            }
        }
    }
    let touched_specs: Vec<_> = paths
        .iter()
        .filter(|path| path.starts_with("openspec/specs/") && path.ends_with(".md"))
        .collect();
    if !touched_specs.is_empty() {
        if associated_ids.is_empty() {
            return Err(
                "main OpenSpec specs changed without an associated archived change; set OpenSpec: change-id"
                    .into(),
            );
        }
        for path in touched_specs {
            let relative = path.strip_prefix("openspec/specs/").unwrap();
            let matching_deltas: Vec<_> = associated_ids
                .iter()
                .flat_map(|associated_id| {
                    files.iter().filter_map(move |(candidate, contents)| {
                        (change_id(candidate) == Some(associated_id.as_str())
                            && candidate.ends_with(&format!("/specs/{relative}")))
                        .then_some((candidate, contents))
                    })
                })
                .collect();
            if matching_deltas.is_empty() {
                return Err(format!(
                    "{path} changed without a matching delta in its associated archived change"
                ));
            }
            let current = files
                .get(path)
                .ok_or_else(|| format!("missing current main spec {path}"))?;
            let base_revision = merge_base.as_deref().unwrap_or("HEAD");
            let previous = git_show_optional(base_revision, path)?.unwrap_or_default();
            if !previous.trim().is_empty()
                && non_requirement_text(&previous) != non_requirement_text(current)
            {
                return Err(format!(
                    "{path} changed outside requirement blocks; record that contract text in an archive delta"
                ));
            }
            let before = requirements(&previous)?;
            let after = requirements(current)?;
            let changed_names: BTreeSet<_> = before
                .keys()
                .chain(after.keys())
                .filter(|name| before.get(*name) != after.get(*name))
                .cloned()
                .collect();
            let mut covered_names = BTreeSet::new();
            for (_, delta) in matching_deltas {
                covered_names.extend(delta_names(delta)?);
            }
            if let Some(name) = changed_names
                .iter()
                .find(|name| !covered_names.contains(*name))
            {
                return Err(format!(
                    "{path} requirement changed without a matching archived delta: {name}"
                ));
            }
        }
    }
    check(&files, &ids)?;
    if ids.is_empty() {
        println!("OpenSpec completion: not applicable, no associated changes");
    } else {
        println!(
            "OpenSpec completion: synchronized and archived: {}",
            ids.into_iter().collect::<Vec<_>>().join(", ")
        );
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("OpenSpec gate: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    const REQUIREMENT: &str = "### Requirement: Safe write\nSHALL preserve state.\n#### Scenario: Retry\n- **WHEN** retried\n- **THEN** no duplicate\n";
    fn fixture(id: &str) -> Files {
        let root = format!("openspec/changes/archive/2026-09-06-{id}");
        let mut files = Files::new();
        for artifact in ["proposal.md", "design.md", "tasks.md"] {
            files.insert(format!("{root}/{artifact}"), "- [x] Done".into());
        }
        files.insert(
            format!("{root}/specs/safety/spec.md"),
            format!("## ADDED Requirements\n{REQUIREMENT}"),
        );
        files.insert(
            "openspec/specs/safety/spec.md".into(),
            format!("## Requirements\n{REQUIREMENT}"),
        );
        files
    }
    fn ids(names: &str) -> BTreeSet<String> {
        select(&[], names).unwrap()
    }
    #[test]
    fn unrelated_active_change_does_not_block_already_synced_archive() {
        let mut files = fixture("safe-write");
        files.insert(
            "openspec/changes/unrelated/tasks.md".into(),
            "- [ ] Pending".into(),
        );
        assert!(check(&files, &ids("safe-write")).is_ok());
    }
    #[test]
    fn edits_deletions_and_both_rename_paths_select_changes() {
        let paths = [
            "openspec/changes/edited/tasks.md",
            "openspec/changes/deleted/proposal.md",
            "openspec/changes/old/specs/a/spec.md",
            "openspec/changes/new/specs/a/spec.md",
            "openspec/changes/archive/2026-09-06-finished/tasks.md",
        ]
        .map(str::to_owned);
        assert_eq!(
            select(&paths, "explicit").unwrap(),
            ids("edited,deleted,old,new,finished,explicit")
        );
    }
    #[test]
    fn active_or_deleted_only_change_fails() {
        let mut files = fixture("safe-write");
        files.insert(
            "openspec/changes/safe-write/proposal.md".into(),
            "active".into(),
        );
        assert!(check(&files, &ids("safe-write")).is_err());
        assert!(check(&Files::new(), &ids("deleted")).is_err());
    }
    #[test]
    fn skipped_sync_and_changed_scenarios_fail() {
        let mut files = fixture("safe-write");
        files
            .get_mut("openspec/specs/safety/spec.md")
            .unwrap()
            .push_str("Extra scenario text\n");
        assert!(check(&files, &ids("safe-write")).is_err());
        files.remove("openspec/specs/safety/spec.md");
        assert!(check(&files, &ids("safe-write")).is_err());
    }
    #[test]
    fn multiple_associations_all_must_pass() {
        let mut files = fixture("first");
        files.extend(fixture("second"));
        assert!(check(&files, &ids("first,second")).is_ok());
        files.insert(
            "openspec/changes/archive/2026-09-06-second/tasks.md".into(),
            "- [ ] Pending".into(),
        );
        assert!(check(&files, &ids("first,second")).is_err());
    }
    #[test]
    fn removals_and_renames_check_final_state() {
        let mut files = fixture("safe-write");
        let delta = "openspec/changes/archive/2026-09-06-safe-write/specs/safety/spec.md";
        let delta_body = format!(
            "## REMOVED Requirements\n### Requirement: Old write\nReason: replaced\n\
             ## MODIFIED Requirements\n{REQUIREMENT}\n## RENAMED Requirements\n\
             - FROM: `### Requirement: Old write`\n- TO: `### Requirement: Safe write`\n"
        );
        files.insert(delta.into(), delta_body);
        assert!(check(&files, &ids("safe-write")).is_ok());
        files
            .get_mut("openspec/specs/safety/spec.md")
            .unwrap()
            .push_str("### Requirement: Old write\nstale\n");
        assert!(check(&files, &ids("safe-write")).is_err());
    }
    #[test]
    fn no_delta_change_requires_explanation() {
        let mut files = fixture("safe-write");
        files.remove("openspec/changes/archive/2026-09-06-safe-write/specs/safety/spec.md");
        assert!(check(&files, &ids("safe-write")).is_err());
        files.insert(
            "openspec/changes/archive/2026-09-06-safe-write/no-spec-deltas.md".into(),
            "Documentation-only correction; no contract changes.".into(),
        );
        assert!(check(&files, &ids("safe-write")).is_ok());
    }

    #[test]
    fn malformed_archive_paths_cannot_skip_selection() {
        assert!(
            select(
                &["openspec/changes/archive/undated-change/tasks.md".into()],
                "none"
            )
            .is_err()
        );
        assert!(select(&["openspec/changes/archive/foo".into()], "none").is_err());
    }

    #[test]
    fn empty_rename_section_is_not_a_delta() {
        let mut files = fixture("safe-write");
        files.insert(
            "openspec/changes/archive/2026-09-06-safe-write/specs/safety/spec.md".into(),
            "## RENAMED Requirements\n".into(),
        );
        assert!(check(&files, &ids("safe-write")).is_err());
    }

    #[test]
    fn direct_main_spec_edits_require_an_associated_change() {
        let paths = vec!["openspec/specs/safety/spec.md".to_owned()];
        assert!(require_association_for_main_specs(&paths, &BTreeSet::new()).is_err());
        assert!(require_association_for_main_specs(&paths, &ids("safe-write")).is_ok());
    }

    #[test]
    fn association_parser_rejects_empty_values_and_accepts_none() {
        assert!(parse_association("").is_err());
        assert!(parse_association(",").is_err());
        assert!(parse_association("none").unwrap().is_empty());
        assert_eq!(parse_association("safe-write").unwrap(), ids("safe-write"));
        assert_eq!(
            parse_association("safe-write, another-change").unwrap(),
            ids("safe-write,another-change")
        );
    }

    #[test]
    fn run_checks_base_ref_and_local_worktree_modes() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let repo = env::temp_dir().join(format!("taku-openspec-{suffix}"));
        fs::create_dir_all(repo.join("openspec/specs/safety")).unwrap();
        let run_git = |args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(&repo)
                .status()
                .unwrap();
            assert!(status.success(), "git command failed: {args:?}");
        };
        run_git(&["init", "--quiet"]);
        run_git(&["config", "user.email", "test@example.com"]);
        run_git(&["config", "user.name", "OpenSpec test"]);
        fs::write(
            repo.join("openspec/specs/safety/spec.md"),
            format!("## Requirements\n{REQUIREMENT}"),
        )
        .unwrap();
        run_git(&["add", "."]);
        run_git(&["commit", "--quiet", "-m", "base"]);

        let archive = repo.join("openspec/changes/archive/2026-09-12-safe-write");
        fs::create_dir_all(archive.join("specs/safety")).unwrap();
        for artifact in ["proposal.md", "design.md"] {
            fs::write(archive.join(artifact), "Done\n").unwrap();
        }
        fs::write(archive.join("tasks.md"), "- [x] Done\n").unwrap();
        fs::write(
            archive.join("specs/safety/spec.md"),
            format!("## MODIFIED Requirements\n{REQUIREMENT}"),
        )
        .unwrap();
        run_git(&["add", "."]);
        run_git(&["commit", "--quiet", "-m", "archive"]);

        let binary = repo.join("check-openspec");
        let source = env::current_dir()
            .unwrap()
            .join("scripts/check-openspec.rs");
        let status = Command::new("rustc")
            .args(["--edition", "2024"])
            .arg(&source)
            .args(["-o"])
            .arg(&binary)
            .status()
            .unwrap();
        assert!(status.success());
        let output = Command::new(&binary)
            .current_dir(&repo)
            .env("BASE_REF", "HEAD~1")
            .env("PR_BODY", "OpenSpec: safe-write")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "base-ref gate failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        fs::write(
            repo.join("openspec/specs/safety/spec.md"),
            format!("## Requirements\n{REQUIREMENT}\n"),
        )
        .unwrap();
        fs::write(archive.join("tasks.md"), "- [x] Done\nLocal note\n").unwrap();
        let output = Command::new(&binary)
            .current_dir(&repo)
            .env_remove("BASE_REF")
            .env_remove("PR_BODY")
            .env_remove("OPENSPEC_CHANGES")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "local gate failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        fs::remove_dir_all(repo).unwrap();
    }
}
