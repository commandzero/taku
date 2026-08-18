use clap::{Command, CommandFactory};
use clap_complete::engine::{ArgValueCompleter, CompletionCandidate as ClapCandidate};
use resource_control::{
    CompletionIntent, CompletionQuery, completion_candidates as engine_candidates,
};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::PathBuf;

pub fn environment() -> ArgValueCompleter {
    ArgValueCompleter::new(|current: &OsStr| complete(CompletionIntent::Environment, current))
}

pub fn application() -> ArgValueCompleter {
    ArgValueCompleter::new(|current: &OsStr| {
        let context = Context::from_process();
        let intent = match (context.command.as_deref(), context.subcommand.as_deref()) {
            (Some("install"), _) => CompletionIntent::InstallApplication,
            (Some("update"), _) => CompletionIntent::UpdateApplication,
            (Some("target"), Some("add")) => CompletionIntent::TargetApplication,
            _ => return Vec::new(),
        };
        let Some(prefix) = current.to_str() else {
            return Vec::new();
        };
        to_clap(
            engine_candidates(&CompletionQuery {
                project: context.project,
                intent,
                environment: context.environment,
                target: None,
                resource_type: None,
                namespace: None,
                prefix: prefix.to_owned(),
                selected: context.positionals,
                provider: context.provider,
            })
            .unwrap_or_default(),
        )
    })
}

pub fn target() -> ArgValueCompleter {
    ArgValueCompleter::new(|current: &OsStr| complete(CompletionIntent::Target, current))
}

pub fn promotion_source_target() -> ArgValueCompleter {
    ArgValueCompleter::new(|current: &OsStr| {
        let mut context = Context::from_process();
        context.environment = context.from_environment.take();
        complete_with(context, CompletionIntent::PromotionTarget, current)
    })
}

pub fn promotion_destination_target() -> ArgValueCompleter {
    ArgValueCompleter::new(|current: &OsStr| {
        let mut context = Context::from_process();
        context.environment = context.to_environment.take();
        complete_with(context, CompletionIntent::PromotionTarget, current)
    })
}

pub fn resource_type() -> ArgValueCompleter {
    ArgValueCompleter::new(|current: &OsStr| {
        let context = Context::from_process();
        let remote = context.command.as_deref() == Some("add")
            || (context.command.as_deref() == Some("list") && context.remote);
        let intent = if remote {
            CompletionIntent::RemoteResourceType
        } else {
            CompletionIntent::LocalResourceType {
                include_markers: matches!(
                    context.command.as_deref(),
                    Some("forget" | "fetch" | "status" | "diff" | "pull" | "push")
                ),
            }
        };
        complete_with(context, intent, current)
    })
}

pub fn resource_id() -> ArgValueCompleter {
    ArgValueCompleter::new(|current: &OsStr| {
        let context = Context::from_process();
        let intent = if context.command.as_deref() == Some("add") {
            CompletionIntent::RemoteResourceId {
                untracked_only: true,
            }
        } else if context.command.as_deref() == Some("list") && context.remote {
            CompletionIntent::RemoteResourceId {
                untracked_only: context.untracked,
            }
        } else {
            CompletionIntent::LocalResourceId {
                include_markers: matches!(
                    context.command.as_deref(),
                    Some("forget" | "fetch" | "status" | "diff" | "pull" | "push")
                ),
            }
        };
        complete_with(context, intent, current)
    })
}

pub fn namespace() -> ArgValueCompleter {
    ArgValueCompleter::new(|current: &OsStr| {
        let context = Context::from_process();
        let remote = context.command.as_deref() == Some("add")
            || (context.command.as_deref() == Some("list") && context.remote);
        complete_with(context, CompletionIntent::Namespace { remote }, current)
    })
}

pub fn provider_key() -> ArgValueCompleter {
    ArgValueCompleter::new(|current: &OsStr| complete(CompletionIntent::ProviderKey, current))
}

fn complete(intent: CompletionIntent, current: &OsStr) -> Vec<ClapCandidate> {
    complete_with(Context::from_process(), intent, current)
}

fn complete_with(
    context: Context,
    intent: CompletionIntent,
    current: &OsStr,
) -> Vec<ClapCandidate> {
    let Some(prefix) = current.to_str() else {
        return Vec::new();
    };
    let query = CompletionQuery {
        project: context.project,
        intent,
        environment: context.environment,
        target: context.positionals.first().cloned(),
        resource_type: context.positionals.get(1).cloned(),
        namespace: context.namespace,
        prefix: prefix.to_owned(),
        selected: context.positionals.into_iter().skip(2).collect(),
        provider: context.provider,
    };
    to_clap(engine_candidates(&query).unwrap_or_default())
}

fn to_clap(candidates: Vec<resource_control::CompletionCandidate>) -> Vec<ClapCandidate> {
    candidates
        .into_iter()
        .map(|candidate| {
            ClapCandidate::new(candidate.value).help(candidate.description.map(Into::into))
        })
        .collect()
}

#[derive(Default)]
struct Context {
    project: PathBuf,
    command: Option<String>,
    subcommand: Option<String>,
    environment: Option<String>,
    from_environment: Option<String>,
    to_environment: Option<String>,
    namespace: Option<String>,
    provider: BTreeMap<String, String>,
    positionals: Vec<String>,
    remote: bool,
    untracked: bool,
    all_environments: bool,
}

impl Context {
    fn from_process() -> Self {
        let all: Vec<String> = std::env::args().collect();
        let start = all
            .iter()
            .rposition(|word| word == "--")
            .map_or(1, |index| index + 1);
        Self::from_words(&all[start..])
    }

    fn from_words(words: &[String]) -> Self {
        let mut context = Self {
            project: PathBuf::from("."),
            ..Self::default()
        };
        let commands = [
            "init",
            "app",
            "target",
            "install",
            "update",
            "context",
            "list",
            "add",
            "remove",
            "forget",
            "promote",
            "validate",
            "fetch",
            "status",
            "diff",
            "pull",
            "push",
            "completion",
        ];
        let mut index = 0;
        while index < words.len() {
            let word = &words[index];
            if let Some(value) = word.strip_prefix("--project=") {
                context.project = PathBuf::from(value);
            } else if word == "--project" {
                index += 1;
                if let Some(value) = words.get(index) {
                    context.project = PathBuf::from(value);
                }
            } else if let Some(value) = option_value(word, "environment") {
                context.environment = Some(value);
            } else if word == "--environment" {
                index += 1;
                context.environment = words.get(index).cloned();
            } else if let Some(value) = option_value(word, "from") {
                context.from_environment = Some(value);
            } else if word == "--from" {
                index += 1;
                context.from_environment = words.get(index).cloned();
            } else if let Some(value) = option_value(word, "to") {
                context.to_environment = Some(value);
            } else if word == "--to" {
                index += 1;
                context.to_environment = words.get(index).cloned();
            } else if let Some(value) = option_value(word, "namespace") {
                context.namespace = Some(value);
            } else if word == "--namespace" {
                index += 1;
                context.namespace = words.get(index).cloned();
            } else if let Some(value) = option_value(word, "set") {
                insert_provider(&mut context.provider, &value);
            } else if word == "--set" {
                index += 1;
                if let Some(value) = words.get(index) {
                    insert_provider(&mut context.provider, value);
                }
            } else if word == "--all-environments" {
                context.all_environments = true;
            } else if word == "--remote" {
                context.remote = true;
            } else if word == "--untracked" {
                if context.command.as_deref() == Some("list") {
                    context.untracked = true;
                } else {
                    index += 1;
                }
            } else if context.command.is_none() && commands.contains(&word.as_str()) {
                context.command = Some(word.clone());
            } else if matches!(
                context.command.as_deref(),
                Some("app" | "target" | "context")
            ) && context.subcommand.is_none()
            {
                context.subcommand = Some(word.clone());
            } else if word.starts_with('-') {
                if option_takes_value(word) && !word.contains('=') {
                    index += 1;
                }
            } else if context.command.is_some() {
                context.positionals.push(word.clone());
            }
            index += 1;
        }
        context
    }
}

fn option_value(word: &str, name: &str) -> Option<String> {
    word.strip_prefix(&format!("--{name}=")).map(str::to_owned)
}

fn option_takes_value(word: &str) -> bool {
    matches!(
        word,
        "--output"
            | "--layout"
            | "--environments"
            | "--from"
            | "--to"
            | "--from-target"
            | "--to-target"
            | "--from-project"
            | "--to-project"
            | "--url"
            | "--missing"
            | "--uncommitted"
    )
}

fn insert_provider(provider: &mut BTreeMap<String, String>, value: &str) {
    if let Some((key, value)) = value.split_once('=') {
        provider.insert(key.to_owned(), value.to_owned());
    }
}

pub fn command() -> Command {
    let context = Context::from_process();
    let Some(command_name) = context.command.as_deref() else {
        return crate::Cli::command();
    };
    let resource_command = matches!(
        command_name,
        "list" | "add" | "remove" | "forget" | "fetch" | "status" | "diff" | "pull" | "push"
    );
    let has_resource_path = resource_command && !context.positionals.is_empty();
    let exact_environment = has_resource_path
        || matches!(command_name, "add" | "remove" | "forget" | "target")
        || (command_name == "list" && context.remote);
    let hide_environment =
        context.all_environments || (exact_environment && context.environment.is_some());
    let hide_all_environments = exact_environment || context.environment.is_some();

    crate::Cli::command().mut_subcommand(command_name, move |command| {
        command.mut_args(|arg| match arg.get_id().as_str() {
            "environment" if hide_environment => arg.hide(true),
            "all_environments" if hide_all_environments => arg.hide(true),
            _ => arg,
        })
    })
}
