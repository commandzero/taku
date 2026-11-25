mod batch;
mod deletion;
mod groups;
mod resource;

use super::journal::{PushJournal, prepare_journal, record_success};
use super::prepare::{PushPlan, PushScope};
use super::{GroupKey, PreparedDeletion, PreparedResource, deletion_report, marker_group};
use crate::reconcile::PushResult;
use crate::transport::RemoteResult;
use crate::{Cardinality, ConcurrencyClass, ResourceType, WriteIntent};
use anyhow::{Result, bail};
use groups::{DependencyState, Groups};
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub(super) fn run(scope: &PushScope<'_>, plan: &mut PushPlan<'_>, new_plan: bool) -> Result<()> {
    let plan_binding = plan.binding();
    let total = plan.resources.len() + plan.deletions.len();
    let journal_path = scope
        .root
        .join(".taku/journals")
        .join(scope.environment)
        .join("push.yaml");
    let journal = prepare_journal(&journal_path, &plan_binding, total, new_plan)?;
    let executor = Executor {
        root: scope.root,
        environment: scope.environment,
        journal_path,
        journal: Mutex::new(journal),
        serial: Mutex::new(()),
        persistence: Mutex::new(()),
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(scope.project.max_requests)
        .build()?;
    let mut dependencies = DependencyState::new(std::mem::take(&mut plan.failed));
    executor.run_writes(&pool, plan, &mut dependencies)?;
    executor.run_deletions(
        std::mem::take(&mut plan.deletions),
        &mut dependencies,
        &mut plan.reports,
    )
}

struct Executor<'a> {
    root: &'a Path,
    environment: &'a str,
    journal_path: PathBuf,
    journal: Mutex<PushJournal>,
    serial: Mutex<()>,
    persistence: Mutex<()>,
}

enum WriteJob {
    One(Box<PreparedResource>),
    Many(Vec<PreparedResource>),
}

impl Executor<'_> {
    fn run_writes(
        &self,
        pool: &rayon::ThreadPool,
        plan: &mut PushPlan<'_>,
        dependencies: &mut DependencyState,
    ) -> Result<()> {
        let mut groups = Groups::new(
            std::mem::take(&mut plan.resources),
            PreparedResource::group,
            |task, key| dependency_groups(key, &task.resource_type, &task.effective_resource_types),
        );
        let reports = &mut plan.reports;
        while !groups.is_empty() {
            groups.remove_blocked(dependencies, |task| {
                reports.push(task.report(self.environment, "blocked_dependency"));
            });
            let ready = groups.ready(dependencies);
            if ready.is_empty() && !groups.is_empty() {
                bail!("Resource Type dependency cycle prevents Push execution");
            }
            let jobs = groups.write_jobs(&ready);
            let results: Vec<_> = pool.install(|| {
                jobs.into_par_iter()
                    .map(|(key, job)| (key, self.run_job(job)))
                    .collect()
            });
            record_write_groups(dependencies, reports, ready, results);
        }
        Ok(())
    }

    fn run_job(&self, job: WriteJob) -> Vec<PushResult> {
        match job {
            WriteJob::One(task) => {
                let fallback = task.report(self.environment, "failed");
                vec![self.write_one(&task).unwrap_or(fallback)]
            }
            WriteJob::Many(tasks) => {
                let fallback = tasks
                    .iter()
                    .map(|task| task.report(self.environment, "failed"))
                    .collect();
                self.write_batch(tasks).unwrap_or(fallback)
            }
        }
    }

    fn run_deletions(
        &self,
        tasks: Vec<PreparedDeletion>,
        dependencies: &mut DependencyState,
        reports: &mut Vec<PushResult>,
    ) -> Result<()> {
        let mut groups = Groups::new(
            tasks,
            |task| marker_group(&task.marker),
            |task, key| dependency_groups(key, &task.resource_type, &task.effective_resource_types),
        );
        while !groups.is_empty() {
            groups.remove_blocked(dependencies, |task| {
                reports.push(deletion_report(
                    self.environment,
                    &task.marker,
                    "blocked_dependency",
                ));
            });
            let ready = groups.ready(dependencies);
            if ready.is_empty() && !groups.is_empty() {
                bail!("Resource Type dependency cycle prevents deletion execution");
            }
            for key in ready {
                let mut ok = true;
                for task in groups.remove(&key) {
                    let fallback = deletion_report(self.environment, &task.marker, "failed");
                    let result = self.delete(&task).unwrap_or(fallback);
                    ok &= matches!(
                        result.outcome.as_str(),
                        "deleted" | "already_absent" | "resumed_success"
                    );
                    reports.push(result);
                }
                dependencies.record(key, ok);
            }
        }
        Ok(())
    }

    fn completed(&self, key: &str) -> bool {
        self.journal.lock().unwrap().completed.contains_key(key)
    }

    fn record_success(&self, key: &str) -> Result<()> {
        record_success(&self.journal_path, &self.journal, key)
    }

    fn mutation(
        &self,
        concurrency: ConcurrencyClass,
        retry_safe: bool,
        request: impl Fn() -> Result<RemoteResult>,
    ) -> Result<RemoteResult> {
        let attempt = || {
            let _guard =
                (concurrency == ConcurrencyClass::Serial).then(|| self.serial.lock().unwrap());
            request()
        };
        let attempts = if retry_safe { 3 } else { 1 };
        let mut result = attempt()?;
        for _ in 1..attempts {
            if !matches!(result, RemoteResult::Retryable | RemoteResult::Uncertain) {
                break;
            }
            result = attempt()?;
        }
        Ok(result)
    }
}

impl Groups<PreparedResource> {
    fn write_jobs(&mut self, ready: &[GroupKey]) -> Vec<(GroupKey, WriteJob)> {
        let mut jobs = Vec::new();
        for key in ready {
            let tasks = self.remove(key);
            if tasks
                .first()
                .is_some_and(|task| task.operation.cardinality == Cardinality::Many)
            {
                jobs.push((key.clone(), WriteJob::Many(tasks)));
            } else {
                jobs.extend(
                    tasks
                        .into_iter()
                        .map(|task| (key.clone(), WriteJob::One(Box::new(task)))),
                );
            }
        }
        jobs
    }
}

fn record_write_groups(
    dependencies: &mut DependencyState,
    reports: &mut Vec<PushResult>,
    ready: Vec<GroupKey>,
    results: Vec<(GroupKey, Vec<PushResult>)>,
) {
    let mut group_ok: BTreeMap<_, _> = ready.into_iter().map(|key| (key, true)).collect();
    for (key, job_reports) in results {
        if job_reports.iter().any(|result| {
            !matches!(
                result.outcome.as_str(),
                "success" | "resumed_success" | "in_sync"
            )
        }) {
            group_ok.insert(key, false);
        }
        reports.extend(job_reports);
    }
    for (key, ok) in group_ok {
        dependencies.record(key, ok);
    }
}

fn dependency_groups(
    (target, namespace, _): &GroupKey,
    resource_type: &ResourceType,
    types: &BTreeMap<String, ResourceType>,
) -> Vec<GroupKey> {
    resource_type
        .dependencies
        .iter()
        .map(|dependency| {
            let namespace = namespace
                .clone()
                .filter(|_| types.get(dependency).is_some_and(|kind| kind.namespaced));
            (target.clone(), namespace, dependency.clone())
        })
        .collect()
}

fn failed_mutation(remote: RemoteResult, intent: WriteIntent) -> &'static str {
    match remote {
        RemoteResult::Conflict => "conflict",
        RemoteResult::Uncertain | RemoteResult::Retryable if intent == WriteIntent::Create => {
            "creation_conflict"
        }
        RemoteResult::Uncertain => "uncertain",
        RemoteResult::Retryable | RemoteResult::Failure(_) => "failed",
        RemoteResult::NotFound => "not_found",
        RemoteResult::Success(_) => unreachable!("successful writes are persisted separately"),
    }
}
