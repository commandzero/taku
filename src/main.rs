use anyhow::{Result, bail};
mod completion;
mod scope;

use clap::{Args, Parser, Subcommand, ValueEnum};
use resource_control::{
    GitPolicy, MissingPolicy, RepositoryLayout, add_remote, add_target, compare, diff, fetch,
    forget, git_root, initialize, install_applications, install_applications_from,
    is_transformation_conflict, list_applications, list_inventory, list_targets, promote,
    promote_projects, pull, push, push_confirmation_required, refresh_source, remote_list, remove,
    rename_target, save_context, update_applications, validate_project,
};
use scope::{EnvironmentSelection, ResourcePath, ResourceScope, ScopePolicy};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "taku",
    version,
    about = "Git-versioned control for remote resources",
    help_template = "\
{about-with-newline}
{usage-heading} {usage}

taku configuration:
  init        Initialize a Taku Project at the exact Git worktree root
  app         List known Applications
  target      List or manage Environment-specific Targets
  install     Vendor Application definitions without creating Targets
  update      Explicitly update vendored Application definitions
  context     Select the local current Environment
  validate    Validate Project metadata, Applications, Targets, and Resources
  completion  Generate a sourceable dynamic shell completion script
  help        Print this message or the help of the given subcommand(s)

Resource management:
  list        List the local Resource Inventory
  add         Adopt explicitly selected remotely listed Resources
  remove      Replace selected Resources with guarded Deletion Markers
  forget      Stop managing selected Resources or Deletion Markers
  promote     Copy complete Resources through Environment mappings
  fetch       Observe selected managed Resources without changing desired state
  status      Summarize desired and Observed State without network access
  diff        Show detailed desired/Observed State differences without network access
  pull        Accept safe Observed State changes into the working tree
  push        Reconcile selected desired Resources to their Targets

Options:
{options}"
)]
struct Cli {
    #[arg(long, global = true, default_value = ".", value_hint = clap::ValueHint::DirPath)]
    project: PathBuf,
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Yaml)]
    output: OutputFormat,
    #[arg(long, global = true)]
    non_interactive: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Args, Clone, Debug, Default)]
struct SingleEnvironmentArgs {
    #[arg(long, add = completion::environment())]
    environment: Option<String>,
}

#[derive(Args, Clone, Debug, Default)]
struct BroadEnvironmentArgs {
    #[arg(long, action = clap::ArgAction::Append, add = completion::environment())]
    environment: Vec<String>,
    #[arg(long, conflicts_with = "environment")]
    all_environments: bool,
}

#[derive(Args, Clone, Debug, Default)]
struct ProviderArgs {
    #[arg(long = "set", value_parser = parse_key_value, action = clap::ArgAction::Append, add = completion::provider_key())]
    values: Vec<(String, String)>,
}

#[derive(Args, Clone, Debug, Default)]
struct PartialPathArgs {
    #[arg(add = completion::target())]
    target: Option<String>,
    #[arg(requires = "target", add = completion::resource_type())]
    resource_type: Option<String>,
    #[arg(requires = "resource_type", add = completion::resource_id())]
    ids: Vec<String>,
}

#[derive(Args, Clone, Debug)]
struct ExactPathArgs {
    #[arg(add = completion::target())]
    target: String,
    #[arg(add = completion::resource_type())]
    resource_type: String,
    #[arg(required = true, num_args = 1.., add = completion::resource_id())]
    ids: Vec<String>,
}

#[derive(Args, Clone, Debug, Default)]
struct PartialResourceArgs {
    #[command(flatten)]
    environment: BroadEnvironmentArgs,
    #[command(flatten)]
    path: PartialPathArgs,
    #[arg(long, requires = "resource_type", add = completion::namespace())]
    namespace: Option<String>,
}

#[derive(Args, Clone, Debug)]
struct ExactResourceArgs {
    #[command(flatten)]
    environment: SingleEnvironmentArgs,
    #[command(flatten)]
    path: ExactPathArgs,
    #[arg(long, add = completion::namespace())]
    namespace: Option<String>,
}

impl PartialResourceArgs {
    fn scope(self) -> Result<ResourceScope> {
        Ok(ResourceScope::new(
            EnvironmentSelection {
                names: self.environment.environment,
                all: self.environment.all_environments,
            },
            ResourcePath::partial(self.path.target, self.path.resource_type, self.path.ids)?,
            self.namespace,
        ))
    }
}

impl ExactResourceArgs {
    fn scope(self) -> Result<ResourceScope> {
        Ok(ResourceScope::new(
            EnvironmentSelection {
                names: self.environment.environment.into_iter().collect(),
                all: false,
            },
            Some(ResourcePath::exact(
                self.path.target,
                self.path.resource_type,
                self.path.ids,
            )?),
            self.namespace,
        ))
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormat {
    Yaml,
    Json,
}

#[derive(Clone, Copy, ValueEnum)]
enum LayoutArg {
    Single,
    Multi,
}

#[derive(Clone, Copy, ValueEnum)]
enum CompletionShell {
    Bash,
    Zsh,
    Fish,
    Elvish,
    Powershell,
}

impl CompletionShell {
    fn as_str(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
            Self::Elvish => "elvish",
            Self::Powershell => "powershell",
        }
    }
}

impl From<LayoutArg> for RepositoryLayout {
    fn from(value: LayoutArg) -> Self {
        match value {
            LayoutArg::Single => Self::Single,
            LayoutArg::Multi => Self::Multi,
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a Taku Project at the exact Git worktree root.
    Init {
        #[arg(long, value_enum)]
        layout: Option<LayoutArg>,
        #[arg(long = "environment", action = clap::ArgAction::Append)]
        environment: Vec<String>,
        #[arg(long = "environments", value_delimiter = ',')]
        environments: Vec<String>,
    },
    /// List known Applications.
    App {
        #[command(subcommand)]
        command: Option<AppCommand>,
    },
    /// List or manage Environment-specific Targets.
    Target {
        #[command(flatten)]
        environment: SingleEnvironmentArgs,
        #[command(subcommand)]
        command: Option<TargetCommand>,
    },
    /// Vendor Application definitions without creating Targets.
    Install {
        #[arg(long)]
        from: Option<String>,
        #[arg(required = true, num_args = 1.., add = completion::application())]
        applications: Vec<String>,
    },
    /// Explicitly update vendored Application definitions.
    Update {
        #[arg(long)]
        from: Option<String>,
        #[arg(add = completion::application())]
        applications: Vec<String>,
    },
    /// Select the local current Environment.
    Context {
        #[command(subcommand)]
        command: ContextCommand,
    },
    /// List the local Resource Inventory.
    List {
        #[command(flatten)]
        scope: PartialResourceArgs,
        #[command(flatten)]
        provider: ProviderArgs,
        #[arg(long, requires = "resource_type")]
        remote: bool,
        #[arg(long, requires = "remote")]
        untracked: bool,
    },
    /// Adopt explicitly selected remotely listed Resources.
    Add {
        #[command(flatten)]
        scope: ExactResourceArgs,
        #[command(flatten)]
        provider: ProviderArgs,
    },
    /// Replace selected Resources with guarded Deletion Markers.
    Remove {
        #[command(flatten)]
        scope: ExactResourceArgs,
    },
    /// Stop managing selected Resources or Deletion Markers.
    Forget {
        #[command(flatten)]
        scope: ExactResourceArgs,
    },
    /// Copy complete Resources through Environment mappings.
    Promote {
        #[arg(long)]
        #[arg(add = completion::environment())]
        from: Option<String>,
        #[arg(long)]
        #[arg(add = completion::environment())]
        to: Option<String>,
        #[arg(long)]
        #[arg(add = completion::promotion_source_target())]
        from_target: Option<String>,
        #[arg(long)]
        #[arg(add = completion::promotion_destination_target())]
        to_target: Option<String>,
        #[arg(long, value_hint = clap::ValueHint::DirPath)]
        from_project: Option<PathBuf>,
        #[arg(long, value_hint = clap::ValueHint::DirPath)]
        to_project: Option<PathBuf>,
    },
    /// Validate Project metadata, Applications, Targets, and Resources.
    Validate,
    /// Generate a sourceable dynamic shell completion script.
    Completion { shell: CompletionShell },
    /// Observe selected managed Resources without changing desired state.
    Fetch {
        #[command(flatten)]
        scope: PartialResourceArgs,
        #[command(flatten)]
        provider: ProviderArgs,
    },
    /// Summarize desired and Observed State without network access.
    Status {
        #[command(flatten)]
        scope: PartialResourceArgs,
        #[arg(long)]
        check: bool,
    },
    /// Show detailed desired/Observed State differences without network access.
    Diff {
        #[command(flatten)]
        scope: PartialResourceArgs,
        #[arg(long = "exit-code")]
        exit_code: bool,
    },
    /// Accept safe Observed State changes into the working tree.
    Pull {
        #[command(flatten)]
        scope: PartialResourceArgs,
        #[arg(long)]
        yes: bool,
        #[arg(long, value_enum)]
        missing: Option<MissingArg>,
    },
    /// Reconcile selected desired Resources to their Targets.
    Push {
        #[command(flatten)]
        scope: PartialResourceArgs,
        #[command(flatten)]
        provider: ProviderArgs,
        #[arg(long)]
        dry_run: bool,
        #[arg(long, value_enum)]
        uncommitted: Option<GitPolicyArg>,
        #[arg(long, value_enum)]
        untracked: Option<GitPolicyArg>,
        #[arg(long, value_enum)]
        missing: Option<MissingArg>,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        new_plan: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum GitPolicyArg {
    Block,
    Confirm,
    Allow,
}
impl From<GitPolicyArg> for GitPolicy {
    fn from(v: GitPolicyArg) -> Self {
        match v {
            GitPolicyArg::Block => Self::Block,
            GitPolicyArg::Confirm => Self::Confirm,
            GitPolicyArg::Allow => Self::Allow,
        }
    }
}
#[derive(Clone, Copy, ValueEnum)]
enum MissingArg {
    Conflict,
    Restore,
    Delete,
}
impl From<MissingArg> for MissingPolicy {
    fn from(v: MissingArg) -> Self {
        match v {
            MissingArg::Conflict => Self::Conflict,
            MissingArg::Restore => Self::Restore,
            MissingArg::Delete => Self::Delete,
        }
    }
}

#[derive(Subcommand)]
enum AppCommand {
    Refresh {
        #[arg(long)]
        from: Option<String>,
    },
}

#[derive(Subcommand)]
enum TargetCommand {
    Add {
        #[arg(add = completion::application())]
        application: String,
        name: Option<String>,
        #[arg(long)]
        url: String,
        #[arg(long)]
        yes: bool,
    },
    Rename {
        #[arg(add = completion::target())]
        old: String,
        new: String,
    },
}

#[derive(Subcommand)]
enum ContextCommand {
    Set {
        #[arg(add = completion::environment())]
        environment: String,
    },
}

#[derive(Serialize)]
struct Envelope<T> {
    schema_version: u32,
    command: &'static str,
    result: T,
}

fn main() {
    clap_complete::CompleteEnv::with_factory(completion::command).complete();
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(2);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Init {
            layout,
            environment,
            mut environments,
        } => {
            let interactive = !cli.non_interactive && io::stdin().is_terminal();
            let layout = match layout {
                Some(value) => value,
                None if interactive => prompt_layout()?,
                None => bail!("--layout is required in non-interactive execution"),
            };
            environments.extend(environment);
            let environments = if environments.is_empty() && interactive {
                vec![prompt("Environment name")?]
            } else if environments.is_empty() {
                bail!("--environment is required in non-interactive execution")
            } else {
                environments
            };
            let result = initialize(&cli.project, layout.into(), environments)?;
            emit(
                cli.output,
                &Envelope {
                    schema_version: 1,
                    command: "init",
                    result,
                },
            )?;
        }
        Commands::App { command: None } => {
            let result = list_applications(&cli.project)?;
            emit(
                cli.output,
                &Envelope {
                    schema_version: 1,
                    command: "app",
                    result,
                },
            )?;
        }
        Commands::Target {
            environment,
            command:
                Some(TargetCommand::Add {
                    application,
                    name,
                    url,
                    yes,
                }),
        } => {
            let name = name.unwrap_or_else(|| application.clone());
            let environment = environment.environment;
            let project_root = git_root(&cli.project)?;
            let installed = project_root
                .join(".taku/applications")
                .join(&application)
                .join("application.yaml")
                .is_file();
            if !installed {
                let authorized = yes
                    || (!cli.non_interactive
                        && io::stdin().is_terminal()
                        && confirm(&format!(
                            "Install Application {application} and add Target {name}"
                        ))?);
                if !authorized {
                    bail!(
                        "Application {application} is not installed; repeat with --yes to install and add atomically"
                    );
                }
                install_applications(&cli.project, std::slice::from_ref(&application))?;
            }
            let added = add_target(
                &cli.project,
                environment.as_deref(),
                &application,
                &name,
                &url,
            );
            if let Err(error) = added {
                if !installed {
                    let installed_path = project_root.join(".taku/applications").join(&application);
                    if let Err(rollback) = std::fs::remove_dir_all(&installed_path) {
                        bail!(
                            "Target Addition failed ({error}); Application install rollback also failed: {rollback}"
                        );
                    }
                }
                return Err(error);
            }
            emit(
                cli.output,
                &Envelope {
                    schema_version: 1,
                    command: "target add",
                    result: serde_json::json!({"environment": environment, "application": application, "target": name}),
                },
            )?;
        }
        Commands::Target {
            environment,
            command: Some(TargetCommand::Rename { old, new }),
        } => {
            rename_target(&cli.project, environment.environment.as_deref(), &old, &new)?;
            emit(
                cli.output,
                &Envelope {
                    schema_version: 1,
                    command: "target rename",
                    result: serde_json::json!({"environment": environment.environment, "old": old, "new": new}),
                },
            )?;
        }
        Commands::App {
            command: Some(AppCommand::Refresh { from }),
        } => {
            let result = refresh_source(&cli.project, from.as_deref())?;
            emit(
                cli.output,
                &Envelope {
                    schema_version: 1,
                    command: "app refresh",
                    result,
                },
            )?;
        }
        Commands::Target {
            environment,
            command: None,
        } => {
            let result = list_targets(&cli.project, environment.environment.as_deref())?;
            emit(
                cli.output,
                &Envelope {
                    schema_version: 1,
                    command: "target",
                    result,
                },
            )?;
        }
        Commands::Install { from, applications } => {
            let result = install_applications_from(&cli.project, &applications, from.as_deref())?;
            emit(
                cli.output,
                &Envelope {
                    schema_version: 1,
                    command: "install",
                    result,
                },
            )?;
        }
        Commands::Update { from, applications } => {
            let result = update_applications(&cli.project, &applications, from.as_deref())?;
            emit(
                cli.output,
                &Envelope {
                    schema_version: 1,
                    command: "update",
                    result,
                },
            )?;
        }
        Commands::Context {
            command: ContextCommand::Set { environment },
        } => {
            let result =
                serde_json::json!({"environment": save_context(&cli.project, &environment)?});
            emit(
                cli.output,
                &Envelope {
                    schema_version: 1,
                    command: "context set",
                    result,
                },
            )?;
        }
        Commands::List {
            scope,
            provider,
            remote,
            untracked,
        } => {
            let scope = scope.scope()?;
            let selections = scope.selections(
                &cli.project,
                if remote {
                    ScopePolicy::RemoteList
                } else {
                    ScopePolicy::Partial
                },
            )?;
            let provider = provider_map(&provider);
            if remote {
                let mut result: Vec<serde_json::Value> = Vec::new();
                let mut transformation_conflict = false;
                for selection in &selections {
                    match remote_list(&cli.project, selection, untracked, &provider) {
                        Ok(entries) => {
                            result.extend(
                                entries
                                    .into_iter()
                                    .map(serde_json::to_value)
                                    .collect::<std::result::Result<Vec<_>, _>>()?,
                            );
                        }
                        Err(error) if is_transformation_conflict(&error) => {
                            transformation_conflict = true;
                            result.push(serde_json::json!({
                                "outcome": "transformation_conflict"
                            }));
                        }
                        Err(error) => return Err(error),
                    }
                }
                emit(
                    cli.output,
                    &Envelope {
                        schema_version: 1,
                        command: "list",
                        result,
                    },
                )?;
                if transformation_conflict {
                    std::process::exit(4);
                }
            } else {
                let mut result = Vec::new();
                for selection in &selections {
                    result.extend(list_inventory(&cli.project, selection)?);
                }
                emit(
                    cli.output,
                    &Envelope {
                        schema_version: 1,
                        command: "list",
                        result,
                    },
                )?;
            }
        }
        Commands::Add { scope, provider } => {
            let selections = scope
                .scope()?
                .selections(&cli.project, ScopePolicy::Exact)?;
            let provider = provider_map(&provider);
            let mut result = Vec::new();
            for selection in &selections {
                result.extend(add_remote(&cli.project, selection, &provider)?);
            }
            emit(
                cli.output,
                &Envelope {
                    schema_version: 1,
                    command: "add",
                    result,
                },
            )?;
        }
        Commands::Remove { scope } => {
            let selections = scope
                .scope()?
                .selections(&cli.project, ScopePolicy::Exact)?;
            let mut result = Vec::new();
            for selection in &selections {
                result.extend(remove(&cli.project, selection)?);
            }
            emit(
                cli.output,
                &Envelope {
                    schema_version: 1,
                    command: "remove",
                    result,
                },
            )?;
        }
        Commands::Forget { scope } => {
            let selections = scope
                .scope()?
                .selections(&cli.project, ScopePolicy::Exact)?;
            let mut result = Vec::new();
            for selection in &selections {
                result.extend(forget(&cli.project, selection)?);
            }
            emit(
                cli.output,
                &Envelope {
                    schema_version: 1,
                    command: "forget",
                    result,
                },
            )?;
        }
        Commands::Promote {
            from,
            to,
            from_target,
            to_target,
            from_project,
            to_project,
        } => {
            let result = if let Some(source) = from_project {
                let destination = to_project.as_deref().unwrap_or(&cli.project);
                promote_projects(
                    &source,
                    destination,
                    from_target.as_deref().ok_or_else(|| {
                        anyhow::anyhow!("--from-target is required for cross-Project Promotion")
                    })?,
                    to_target.as_deref().ok_or_else(|| {
                        anyhow::anyhow!("--to-target is required for cross-Project Promotion")
                    })?,
                )?
            } else {
                promote(
                    &cli.project,
                    from.as_deref(),
                    to.as_deref(),
                    from_target.as_deref(),
                    to_target.as_deref(),
                )?
            };
            emit(
                cli.output,
                &Envelope {
                    schema_version: 1,
                    command: "promote",
                    result,
                },
            )?;
        }
        Commands::Validate => {
            let result = validate_project(&cli.project)?;
            emit(
                cli.output,
                &Envelope {
                    schema_version: 1,
                    command: "validate",
                    result,
                },
            )?;
        }
        Commands::Completion { shell } => {
            let executable = std::env::current_exe()?;
            let output = std::process::Command::new(executable)
                .env("COMPLETE", shell.as_str())
                .output()?;
            if !output.status.success() {
                bail!(
                    "failed to generate {} completion: {}",
                    shell.as_str(),
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            io::stdout().write_all(&output.stdout)?;
        }
        Commands::Fetch { scope, provider } => {
            let selections = scope
                .scope()?
                .selections(&cli.project, ScopePolicy::Partial)?;
            let provider = provider_map(&provider);
            let mut result = Vec::new();
            for selection in &selections {
                result.extend(fetch(&cli.project, selection, &provider)?);
            }
            let conflicts = result
                .iter()
                .any(|item| item.outcome == "transformation_conflict");
            emit(
                cli.output,
                &Envelope {
                    schema_version: 1,
                    command: "fetch",
                    result,
                },
            )?;
            if conflicts {
                std::process::exit(4);
            }
        }
        Commands::Status { scope, check } => {
            let selections = scope
                .scope()?
                .selections(&cli.project, ScopePolicy::Partial)?;
            let mut result = Vec::new();
            for selection in &selections {
                result.extend(compare(&cli.project, selection)?);
            }
            let differs = result.iter().any(|item| item.state != "in_sync");
            let conflicts = result.iter().any(|item| item.state.ends_with("conflict"));
            emit(
                cli.output,
                &Envelope {
                    schema_version: 1,
                    command: "status",
                    result,
                },
            )?;
            if conflicts {
                std::process::exit(4);
            } else if check && differs {
                std::process::exit(3);
            }
        }
        Commands::Diff { scope, exit_code } => {
            let selections = scope
                .scope()?
                .selections(&cli.project, ScopePolicy::Partial)?;
            let mut result = Vec::new();
            for selection in &selections {
                result.extend(diff(&cli.project, selection)?);
            }
            let differs = !result.is_empty();
            let conflicts = result.iter().any(|item| item.state.ends_with("conflict"));
            emit(
                cli.output,
                &Envelope {
                    schema_version: 1,
                    command: "diff",
                    result,
                },
            )?;
            if conflicts {
                std::process::exit(4);
            } else if exit_code && differs {
                std::process::exit(3);
            }
        }
        Commands::Pull {
            scope,
            yes,
            missing,
        } => {
            let selections = scope
                .scope()?
                .selections(&cli.project, ScopePolicy::Partial)?;
            if !yes && !io::stdin().is_terminal() {
                bail!("--yes is required to apply Pull non-interactively");
            }
            if !yes && !confirm("Apply observed changes to desired Resources")? {
                bail!("Pull cancelled");
            }
            let mut result = Vec::new();
            for selection in &selections {
                result.extend(pull(&cli.project, selection, missing.map(Into::into))?);
            }
            let conflicts = result.iter().any(|item| item.outcome.ends_with("conflict"));
            emit(
                cli.output,
                &Envelope {
                    schema_version: 1,
                    command: "pull",
                    result,
                },
            )?;
            if conflicts {
                std::process::exit(4);
            }
        }
        Commands::Push {
            scope,
            provider,
            dry_run,
            uncommitted,
            untracked,
            missing,
            yes,
            new_plan,
        } => {
            let selections = scope
                .scope()?
                .selections(&cli.project, ScopePolicy::Partial)?;
            let provider = provider_map(&provider);
            let interactive = !cli.non_interactive && io::stdin().is_terminal();
            let mut confirmed = yes;
            if interactive && !confirmed {
                let mut required = false;
                for selection in &selections {
                    required |= push_confirmation_required(
                        &cli.project,
                        selection,
                        uncommitted.map(Into::into),
                        untracked.map(Into::into),
                    )?;
                }
                if required {
                    confirmed = confirm("Push selected Resources with pending Git changes")?;
                    if !confirmed {
                        bail!("Push cancelled");
                    }
                }
            }
            let mut result = Vec::new();
            for selection in &selections {
                result.extend(push(
                    &cli.project,
                    selection,
                    dry_run,
                    uncommitted.map(Into::into),
                    untracked.map(Into::into),
                    interactive,
                    confirmed,
                    new_plan,
                    missing.map(Into::into),
                    &provider,
                )?);
            }
            let failed = result.iter().any(|item| {
                !matches!(
                    item.outcome.as_str(),
                    "success"
                        | "resumed_success"
                        | "in_sync"
                        | "planned"
                        | "planned_delete"
                        | "planned_already_absent"
                        | "deleted"
                        | "already_absent"
                )
            });
            emit(
                cli.output,
                &Envelope {
                    schema_version: 1,
                    command: "push",
                    result,
                },
            )?;
            if failed {
                std::process::exit(4);
            }
        }
    }
    Ok(())
}

fn provider_map(args: &ProviderArgs) -> BTreeMap<String, String> {
    args.values.iter().cloned().collect()
}
fn parse_key_value(value: &str) -> Result<(String, String), String> {
    let Some((key, value)) = value.split_once('=') else {
        return Err("expected KEY=VALUE".into());
    };
    if key.is_empty() {
        return Err("provider key may not be empty".into());
    }
    Ok((key.into(), value.into()))
}

fn prompt_layout() -> Result<LayoutArg> {
    let value = prompt("Repository layout (single/multi)")?;
    match value.as_str() {
        "single" => Ok(LayoutArg::Single),
        "multi" => Ok(LayoutArg::Multi),
        _ => bail!("layout must be single or multi"),
    }
}

fn prompt(label: &str) -> Result<String> {
    eprint!("{label}: ");
    io::stderr().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        bail!("{label} is required");
    }
    Ok(value)
}

fn confirm(label: &str) -> Result<bool> {
    eprint!("{label}? [y/N]: ");
    io::stderr().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    Ok(matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn emit<T: Serialize>(format: OutputFormat, value: &T) -> Result<()> {
    let document = match format {
        OutputFormat::Yaml => serde_yaml::to_string(value)?,
        OutputFormat::Json => format!("{}\n", serde_json::to_string_pretty(value)?),
    };
    write_document(&mut io::stdout().lock(), document.as_bytes())?;
    Ok(())
}

fn write_document(writer: &mut impl Write, document: &[u8]) -> io::Result<()> {
    writer.write_all(document)?;
    writer.flush()
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn output_write_errors_propagate_without_panicking() {
        struct ClosedPipe;
        impl Write for ClosedPipe {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::from(io::ErrorKind::BrokenPipe))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert_eq!(
            write_document(&mut ClosedPipe, b"report")
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("taku").chain(args.iter().copied()))
    }

    #[test]
    fn exact_commands_require_a_complete_variadic_resource_path() {
        for command in ["add", "remove", "forget"] {
            assert!(parse(&[command]).is_err());
            assert!(parse(&[command, "es"]).is_err());
            assert!(parse(&[command, "es", "roles"]).is_err());
            assert!(parse(&[command, "es", "roles", "one", "two"]).is_ok());
        }
    }

    #[test]
    fn partial_commands_accept_every_contiguous_path_prefix() {
        for command in ["list", "fetch", "status", "diff", "pull", "push"] {
            assert!(parse(&[command]).is_ok(), "{command}");
            assert!(parse(&[command, "es"]).is_ok(), "{command}");
            assert!(parse(&[command, "es", "roles"]).is_ok(), "{command}");
            assert!(
                parse(&[command, "es", "roles", "one", "two"]).is_ok(),
                "{command}"
            );
        }
    }

    #[test]
    fn remote_list_requires_target_and_type_and_owns_untracked() {
        assert!(parse(&["list", "--remote"]).is_err());
        assert!(parse(&["list", "--remote", "es"]).is_err());
        assert!(parse(&["list", "--remote", "es", "roles"]).is_ok());
        assert!(parse(&["list", "--untracked"]).is_err());
    }

    #[test]
    fn namespace_and_removed_selector_flags_are_rejected_outside_the_new_grammar() {
        assert!(parse(&["status", "--namespace", "default"]).is_err());
        for removed in ["--target", "--type", "--id"] {
            assert!(parse(&["add", removed, "value", "es", "roles", "one"]).is_err());
        }
        assert!(parse(&["app", "add"]).is_err());
        assert!(parse(&["app", "rename"]).is_err());
    }

    #[test]
    fn root_and_command_specific_options_stay_separate() {
        assert!(parse(&["validate", "--environment", "dev"]).is_err());
        assert!(parse(&["app", "--namespace", "default"]).is_err());
        assert!(parse(&["promote", "--set", "token=value"]).is_err());
        assert!(parse(&["fetch", "--environment", "dev", "--set", "token=value"]).is_ok());
    }

    #[test]
    fn remaining_commands_keep_their_concise_grammar() {
        assert!(parse(&["install"]).is_err());
        assert!(parse(&["install", "elasticsearch", "kibana"]).is_ok());
        assert!(parse(&["update"]).is_ok());
        assert!(parse(&["context", "set", "prod"]).is_ok());
        assert!(parse(&["validate"]).is_ok());
        assert!(parse(&["promote", "--from", "dev", "--to", "prod"]).is_ok());
    }
}
