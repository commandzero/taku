use super::GroupKey;
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct DependencyState {
    successful: BTreeSet<GroupKey>,
    failed: BTreeSet<GroupKey>,
}

impl DependencyState {
    pub(super) fn new(failed: BTreeSet<GroupKey>) -> Self {
        Self {
            successful: BTreeSet::new(),
            failed,
        }
    }

    pub(super) fn record(&mut self, key: GroupKey, ok: bool) {
        if ok {
            self.successful.insert(key);
        } else {
            self.failed.insert(key);
        }
    }
}

struct Group<T> {
    dependencies: Vec<GroupKey>,
    tasks: Vec<T>,
}

pub(super) struct Groups<T>(BTreeMap<GroupKey, Group<T>>);

impl<T> Groups<T> {
    pub(super) fn new(
        tasks: Vec<T>,
        key_for: impl Fn(&T) -> GroupKey,
        dependencies_for: impl Fn(&T, &GroupKey) -> Vec<GroupKey>,
    ) -> Self {
        let mut groups = BTreeMap::new();
        for task in tasks {
            let key = key_for(&task);
            groups
                .entry(key.clone())
                .or_insert_with(|| Group {
                    dependencies: dependencies_for(&task, &key),
                    tasks: Vec::new(),
                })
                .tasks
                .push(task);
        }
        Self(groups)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(super) fn remove(&mut self, key: &GroupKey) -> Vec<T> {
        self.0.remove(key).unwrap().tasks
    }

    pub(super) fn remove_blocked(
        &mut self,
        state: &mut DependencyState,
        mut report: impl FnMut(T),
    ) {
        let blocked: Vec<_> = self
            .0
            .iter()
            .filter(|(_, group)| {
                group
                    .dependencies
                    .iter()
                    .any(|dependency| state.failed.contains(dependency))
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in blocked {
            self.remove(&key).into_iter().for_each(&mut report);
            state.record(key, false);
        }
    }

    pub(super) fn ready(&self, state: &DependencyState) -> Vec<GroupKey> {
        self.0
            .iter()
            .filter(|(_, group)| {
                group.dependencies.iter().all(|dependency| {
                    state.successful.contains(dependency) || !self.0.contains_key(dependency)
                })
            })
            .map(|(key, _)| key.clone())
            .collect()
    }
}
