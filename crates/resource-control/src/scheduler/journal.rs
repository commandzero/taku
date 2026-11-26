use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PushJournal {
    schema_version: u32,
    binding: String,
    total: usize,
    pub(super) completed: BTreeMap<String, String>,
}

pub(super) fn prepare_journal(
    path: &Path,
    binding: &str,
    total: usize,
    new_plan: bool,
) -> Result<PushJournal> {
    if let Ok(text) = fs::read_to_string(path) {
        let existing: PushJournal = serde_yaml::from_str(&text).context("invalid Push Journal")?;
        if existing.schema_version != crate::SCHEMA_VERSION {
            bail!(
                "unsupported Push Journal schema version {}",
                existing.schema_version
            );
        }
        if existing.binding == binding {
            return Ok(existing);
        }
        if existing.completed.len() < existing.total && !new_plan {
            bail!(
                "an incomplete Push Journal has different inputs; repeat with --new-plan to replace it explicitly"
            );
        }
    }
    let journal = PushJournal {
        schema_version: 1,
        binding: binding.into(),
        total,
        completed: BTreeMap::new(),
    };
    save_journal(path, &journal)?;
    Ok(journal)
}
pub(super) fn record_success(path: &Path, journal: &Mutex<PushJournal>, key: &str) -> Result<()> {
    let mut journal = journal.lock().unwrap();
    journal.completed.insert(key.into(), "success".into());
    save_journal(path, &journal)
}
fn save_journal(path: &Path, journal: &PushJournal) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("next");
    fs::write(&temporary, serde_yaml::to_string(journal)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}
