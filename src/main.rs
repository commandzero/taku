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
use std::path::{Path, PathBuf};

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
    List(ListArgs),
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
    Promote(PromotionArgs),
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
    Push(PushArgs),
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
    Add(TargetAddArgs),
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

#[derive(Args)]
struct ListArgs {
    #[command(flatten)]
    scope: PartialResourceArgs,
    #[command(flatten)]
    provider: ProviderArgs,
    #[arg(long, requires = "resource_type")]
    remote: bool,
    #[arg(long, requires = "remote")]
    untracked: bool,
}

#[derive(Args)]
struct PromotionArgs {
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
}

#[derive(Args)]
struct TargetAddArgs {
    #[arg(add = completion::application())]
    application: String,
    name: Option<String>,
    #[arg(long)]
    url: String,
    #[arg(long)]
    yes: bool,
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
    let options = Cli::parse();
    let cli = CommandContext {
        project: &options.project,
        output: options.output,
        non_interactive: options.non_interactive,
    };
    match options.command {
        Commands::Init {
            layout,
            environment,
            environments,
        } => init_project(&cli, layout, environment, environments)?,
        Commands::App { command } => match command {
            None => cli.report("app", list_applications(cli.project)?)?,
            Some(AppCommand::Refresh { from }) => {
                cli.report("app refresh", refresh_source(cli.project, from.as_deref())?)?;
            }
        },
        Commands::Target {
            environment,
            command,
        } => manage_target(&cli, environment, command)?,
        Commands::Install { from, applications } => {
            let result = install_applications_from(cli.project, &applications, from.as_deref())?;
            cli.report("install", result)?;
        }
        Commands::Update { from, applications } => {
            let result = update_applications(cli.project, &applications, from.as_deref())?;
            cli.report("update", result)?;
        }
        Commands::Context {
            command: ContextCommand::Set { environment },
        } => {
            let result =
                serde_json::json!({"environment": save_context(cli.project, &environment)?});
            cli.report("context set", result)?;
        }
        Commands::List(args) => list_resources(&cli, args)?,
        Commands::Add { scope, provider } => {
            let provider = provider_map(&provider);
            cli.exact("add", scope, |scope| {
                add_remote(cli.project, scope, &provider)
            })?;
        }
        Commands::Remove { scope } => {
            cli.exact("remove", scope, |scope| remove(cli.project, scope))?
        }
        Commands::Forget { scope } => {
            cli.exact("forget", scope, |scope| forget(cli.project, scope))?
        }
        Commands::Promote(args) => promote_resources(&cli, args)?,
        Commands::Validate => {
            let result = validate_project(cli.project)?;
            cli.report("validate", result)?;
        }
        Commands::Completion { shell } => generate_completion(shell)?,
        Commands::Fetch { scope, provider } => fetch_resources(&cli, scope, provider)?,
        Commands::Status { scope, check } => resource_status(&cli, scope, check)?,
        Commands::Diff { scope, exit_code } => resource_diff(&cli, scope, exit_code)?,
        Commands::Pull {
            scope,
            yes,
            missing,
        } => pull_resources(&cli, scope, yes, missing)?,
        Commands::Push(args) => push_resources(&cli, args)?,
    }
    Ok(())
}

struct CommandContext<'a> {
    project: &'a Path,
    output: OutputFormat,
    non_interactive: bool,
}

impl CommandContext<'_> {
    fn exact<T: Serialize>(
        &self,
        command: &'static str,
        scope: ExactResourceArgs,
        run: impl Fn(&resource_control::Selection) -> Result<Vec<T>>,
    ) -> Result<()> {
        let selections = scope
            .scope()?
            .selections(self.project, ScopePolicy::Exact)?;
        let mut result = Vec::new();
        for selection in &selections {
            result.extend(run(selection)?);
        }
        self.report(command, result)
    }

    fn interactive(&self) -> bool {
        !self.non_interactive && io::stdin().is_terminal()
    }

    fn report(&self, command: &'static str, result: impl Serialize) -> Result<()> {
        emit(
            self.output,
            &Envelope {
                schema_version: 1,
                command,
                result,
            },
        )
    }
}

#[derive(Args)]
struct PushArgs {
    #[command(flatten)]
    scope: PartialResourceArgs,
    #[command(flatten)]
    provider: ProviderArgs,
    #[command(flatten)]
    options: PushOptions,
}

#[derive(Args)]
struct PushOptions {
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
}

fn init_project(
    cli: &CommandContext<'_>,
    layout: Option<LayoutArg>,
    environment: Vec<String>,
    mut environments: Vec<String>,
) -> Result<()> {
    let interactive = cli.interactive();
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
    let result = initialize(cli.project, layout.into(), environments)?;
    cli.report("init", result)?;
    Ok(())
}

fn manage_target(
    cli: &CommandContext<'_>,
    environment: SingleEnvironmentArgs,
    command: Option<TargetCommand>,
) -> Result<()> {
    let environment = environment.environment.as_deref();
    match command {
        Some(TargetCommand::Add(args)) => add_project_target(cli, environment, args),
        Some(TargetCommand::Rename { old, new }) => {
            rename_target(cli.project, environment, &old, &new)?;
            cli.report(
                "target rename",
                serde_json::json!({"environment": environment, "old": old, "new": new}),
            )
        }
        None => cli.report("target", list_targets(cli.project, environment)?),
    }
}

fn add_project_target(
    cli: &CommandContext<'_>,
    environment: Option<&str>,
    args: TargetAddArgs,
) -> Result<()> {
    let TargetAddArgs {
        application,
        name,
        url,
        yes,
    } = args;
    let name = name.unwrap_or_else(|| application.clone());
    let installed_path = git_root(cli.project)?
        .join(".taku/applications")
        .join(&application);
    let installed = installed_path.join("application.yaml").is_file();
    if !installed {
        install_for_target(cli, &application, &name, yes)?;
    }
    if let Err(error) = add_target(cli.project, environment, &application, &name, &url) {
        if !installed {
            rollback_target_install(&installed_path, &error)?;
        }
        return Err(error);
    }
    cli.report(
        "target add",
        serde_json::json!({"environment": environment, "application": application, "target": name}),
    )?;
    Ok(())
}

fn install_for_target(
    cli: &CommandContext<'_>,
    application: &str,
    name: &str,
    yes: bool,
) -> Result<()> {
    let authorized = yes
        || (cli.interactive()
            && confirm(&format!(
                "Install Application {application} and add Target {name}"
            ))?);
    if !authorized {
        bail!(
            "Application {application} is not installed; repeat with --yes to install and add atomically"
        );
    }
    install_applications(cli.project, &[application.to_owned()])?;
    Ok(())
}

fn rollback_target_install(path: &Path, error: &anyhow::Error) -> Result<()> {
    if let Err(rollback) = std::fs::remove_dir_all(path) {
        bail!(
            "Target Addition failed ({error}); Application install rollback also failed: {rollback}"
        );
    }
    Ok(())
}

fn list_resources(cli: &CommandContext<'_>, args: ListArgs) -> Result<()> {
    let ListArgs {
        scope,
        provider,
        remote,
        untracked,
    } = args;
    let scope = scope.scope()?;
    let selections = scope.selections(
        cli.project,
        if remote {
            ScopePolicy::RemoteList
        } else {
            ScopePolicy::Partial
        },
    )?;
    let provider = provider_map(&provider);
    if remote {
        return list_remote_resources(cli, &selections, &provider, untracked);
    }
    let mut result = Vec::new();
    for selection in &selections {
        result.extend(list_inventory(cli.project, selection)?);
    }
    cli.report("list", result)
}

fn list_remote_resources(
    cli: &CommandContext<'_>,
    selections: &[resource_control::Selection],
    provider: &BTreeMap<String, String>,
    untracked: bool,
) -> Result<()> {
    let mut result: Vec<serde_json::Value> = Vec::new();
    let mut transformation_conflict = false;
    for selection in selections {
        match remote_list(cli.project, selection, untracked, provider) {
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
    cli.report("list", result)?;
    if transformation_conflict {
        std::process::exit(4);
    }
    Ok(())
}

fn promote_resources(cli: &CommandContext<'_>, args: PromotionArgs) -> Result<()> {
    let PromotionArgs {
        from,
        to,
        from_target,
        to_target,
        from_project,
        to_project,
    } = args;
    let result = if let Some(source) = from_project {
        let destination = to_project.as_deref().unwrap_or(cli.project);
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
            cli.project,
            from.as_deref(),
            to.as_deref(),
            from_target.as_deref(),
            to_target.as_deref(),
        )?
    };
    cli.report("promote", result)?;
    Ok(())
}

fn generate_completion(shell: CompletionShell) -> Result<()> {
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
    Ok(())
}

fn fetch_resources(
    cli: &CommandContext<'_>,
    scope: PartialResourceArgs,
    provider: ProviderArgs,
) -> Result<()> {
    let selections = scope
        .scope()?
        .selections(cli.project, ScopePolicy::Partial)?;
    let provider = provider_map(&provider);
    let mut result = Vec::new();
    for selection in &selections {
        result.extend(fetch(cli.project, selection, &provider)?);
    }
    let conflicts = result
        .iter()
        .any(|item| item.outcome == "transformation_conflict");
    cli.report("fetch", result)?;
    if conflicts {
        std::process::exit(4);
    }
    Ok(())
}

fn resource_status(
    cli: &CommandContext<'_>,
    scope: PartialResourceArgs,
    check: bool,
) -> Result<()> {
    let selections = scope
        .scope()?
        .selections(cli.project, ScopePolicy::Partial)?;
    let mut result = Vec::new();
    for selection in &selections {
        result.extend(compare(cli.project, selection)?);
    }
    let differs = result.iter().any(|item| item.state != "in_sync");
    let conflicts = result.iter().any(|item| item.state.ends_with("conflict"));
    cli.report("status", result)?;
    if conflicts {
        std::process::exit(4);
    } else if check && differs {
        std::process::exit(3);
    }
    Ok(())
}

fn resource_diff(
    cli: &CommandContext<'_>,
    scope: PartialResourceArgs,
    exit_code: bool,
) -> Result<()> {
    let selections = scope
        .scope()?
        .selections(cli.project, ScopePolicy::Partial)?;
    let mut result = Vec::new();
    for selection in &selections {
        result.extend(diff(cli.project, selection)?);
    }
    let differs = !result.is_empty();
    let conflicts = result.iter().any(|item| item.state.ends_with("conflict"));
    cli.report("diff", result)?;
    if conflicts {
        std::process::exit(4);
    } else if exit_code && differs {
        std::process::exit(3);
    }
    Ok(())
}

fn pull_resources(
    cli: &CommandContext<'_>,
    scope: PartialResourceArgs,
    yes: bool,
    missing: Option<MissingArg>,
) -> Result<()> {
    let selections = scope
        .scope()?
        .selections(cli.project, ScopePolicy::Partial)?;
    if !yes && !io::stdin().is_terminal() {
        bail!("--yes is required to apply Pull non-interactively");
    }
    if !yes && !confirm("Apply observed changes to desired Resources")? {
        bail!("Pull cancelled");
    }
    let mut result = Vec::new();
    for selection in &selections {
        result.extend(pull(cli.project, selection, missing.map(Into::into))?);
    }
    let conflicts = result.iter().any(|item| item.outcome.ends_with("conflict"));
    cli.report("pull", result)?;
    if conflicts {
        std::process::exit(4);
    }
    Ok(())
}

fn push_resources(cli: &CommandContext<'_>, args: PushArgs) -> Result<()> {
    let PushArgs {
        scope,
        provider,
        options,
    } = args;
    let selections = scope
        .scope()?
        .selections(cli.project, ScopePolicy::Partial)?;
    let provider = provider_map(&provider);
    let interactive = cli.interactive();
    let confirmed = confirm_push(cli.project, &selections, &options, interactive)?;
    let mut result = Vec::new();
    for selection in &selections {
        result.extend(push(
            cli.project,
            selection,
            options.dry_run,
            options.uncommitted.map(Into::into),
            options.untracked.map(Into::into),
            interactive,
            confirmed,
            options.new_plan,
            options.missing.map(Into::into),
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
    cli.report("push", result)?;
    if failed {
        std::process::exit(4);
    }
    Ok(())
}

fn confirm_push(
    project: &Path,
    selections: &[resource_control::Selection],
    options: &PushOptions,
    interactive: bool,
) -> Result<bool> {
    if options.yes || !interactive {
        return Ok(options.yes);
    }
    let mut required = false;
    for selection in selections {
        required |= push_confirmation_required(
            project,
            selection,
            options.uncommitted.map(Into::into),
            options.untracked.map(Into::into),
        )?;
    }
    if !required {
        return Ok(false);
    }
    if !confirm("Push selected Resources with pending Git changes")? {
        bail!("Push cancelled");
    }
    Ok(true)
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
mod cli_tests;
