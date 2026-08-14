use anyhow::{Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use resource_control::{
    GitPolicy, MissingPolicy, RepositoryLayout, Selection, add_remote, add_target, compare, diff,
    fetch, forget, git_root, initialize, install_applications, install_applications_from,
    is_transformation_conflict, list_applications, list_inventory, load_project, promote,
    promote_projects, pull, push, push_confirmation_required, refresh_source, remote_list, remove,
    rename_target, save_context, update_applications, validate_project,
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "taku",
    version,
    about = "Git-versioned control for remote resources"
)]
struct Cli {
    #[arg(long, global = true, default_value = ".")]
    project: PathBuf,
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Yaml)]
    output: OutputFormat,
    #[arg(long, global = true)]
    non_interactive: bool,
    #[arg(long, global = true, action = clap::ArgAction::Append)]
    environment: Vec<String>,
    #[arg(long, global = true, conflicts_with = "environment")]
    all_environments: bool,
    #[arg(long, global = true, action = clap::ArgAction::Append)]
    target: Vec<String>,
    #[arg(long, global = true, action = clap::ArgAction::Append)]
    namespace: Vec<String>,
    #[arg(long = "type", global = true, action = clap::ArgAction::Append)]
    resource_type: Vec<String>,
    #[arg(long, global = true, action = clap::ArgAction::Append)]
    id: Vec<String>,
    #[arg(long = "set", global = true, value_parser = parse_key_value, action = clap::ArgAction::Append)]
    provider_values: Vec<(String, String)>,
    #[command(subcommand)]
    command: Commands,
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
        #[arg(long = "environments", value_delimiter = ',')]
        environments: Vec<String>,
    },
    /// List known Applications or manage Environment-specific Targets.
    App {
        #[command(subcommand)]
        command: Option<AppCommand>,
    },
    /// Vendor Application definitions without creating Targets.
    Install {
        #[arg(long)]
        from: Option<String>,
        applications: Vec<String>,
    },
    /// Explicitly update vendored Application definitions.
    Update {
        #[arg(long)]
        from: Option<String>,
        applications: Vec<String>,
    },
    /// Select the local current Environment.
    Context {
        #[command(subcommand)]
        command: ContextCommand,
    },
    /// List the local Resource Inventory.
    List {
        kind: Option<String>,
        #[arg(long)]
        remote: bool,
        #[arg(long, requires = "remote")]
        untracked: bool,
    },
    /// Adopt explicitly selected remotely listed Resources.
    Add,
    /// Replace selected Resources with guarded Deletion Markers.
    Remove,
    /// Stop managing selected Resources or Deletion Markers.
    Forget,
    /// Copy complete Resources through Environment mappings.
    Promote {
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        from_target: Option<String>,
        #[arg(long)]
        to_target: Option<String>,
        #[arg(long)]
        from_project: Option<PathBuf>,
        #[arg(long)]
        to_project: Option<PathBuf>,
    },
    /// Validate Project metadata, Applications, Targets, and Resources.
    Validate,
    /// Observe selected managed Resources without changing desired state.
    Fetch,
    /// Summarize desired and Observed State without network access.
    Status {
        #[arg(long)]
        check: bool,
    },
    /// Show detailed desired/Observed State differences without network access.
    Diff {
        #[arg(long = "exit-code")]
        exit_code: bool,
    },
    /// Accept safe Observed State changes into the working tree.
    Pull {
        #[arg(long)]
        yes: bool,
        #[arg(long, value_enum)]
        missing: Option<MissingArg>,
    },
    /// Reconcile selected desired Resources to their Targets.
    Push {
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
    Add {
        application: String,
        name: Option<String>,
        #[arg(long)]
        url: String,
        #[arg(long)]
        yes: bool,
    },
    Rename {
        old: String,
        new: String,
    },
    Refresh {
        #[arg(long)]
        from: Option<String>,
    },
}

#[derive(Subcommand)]
enum ContextCommand {
    Set { environment: String },
}

#[derive(Serialize)]
struct Envelope<T> {
    schema_version: u32,
    command: &'static str,
    result: T,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(2);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let explicit_environments = cli.environment.clone();
    let common_selections = if matches!(&cli.command, Commands::Init { .. }) {
        Vec::new()
    } else {
        selections(&cli)?
    };
    let common_provider = provider_map(&cli);
    match cli.command {
        Commands::Init {
            layout,
            mut environments,
        } => {
            let interactive = !cli.non_interactive && io::stdin().is_terminal();
            let layout = match layout {
                Some(value) => value,
                None if interactive => prompt_layout()?,
                None => bail!("--layout is required in non-interactive execution"),
            };
            environments.extend(cli.environment.clone());
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
        Commands::App {
            command:
                Some(AppCommand::Add {
                    application,
                    name,
                    url,
                    yes,
                }),
        } => {
            let name = name.unwrap_or_else(|| application.clone());
            let environment = one_environment_names(&explicit_environments, cli.all_environments)?;
            let project_root = git_root(&cli.project)?;
            let installed = project_root
                .join(".taku/applications")
                .join(&application)
                .join("resources.yml")
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
                    command: "app add",
                    result: serde_json::json!({"environment": environment, "application": application, "target": name}),
                },
            )?;
        }
        Commands::App {
            command: Some(AppCommand::Rename { old, new }),
        } => {
            rename_target(
                &cli.project,
                one_environment_names(&explicit_environments, cli.all_environments)?.as_deref(),
                &old,
                &new,
            )?;
            emit(
                cli.output,
                &Envelope {
                    schema_version: 1,
                    command: "app rename",
                    result: serde_json::json!({"environment": one_environment_names(&explicit_environments,cli.all_environments)?, "old": old, "new": new}),
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
            kind,
            remote,
            untracked,
        } => {
            let mut types = cli.resource_type.clone();
            if let Some(resource_type) = kind {
                types.push(resource_type);
            }
            if remote {
                let mut result: Vec<serde_json::Value> = Vec::new();
                let mut transformation_conflict = false;
                for base in &common_selections {
                    let mut scope = base.clone();
                    scope.types = types.clone();
                    match remote_list(&cli.project, &scope, untracked, &common_provider) {
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
                for base in &common_selections {
                    let mut scope = base.clone();
                    scope.types = types.clone();
                    result.extend(list_inventory(&cli.project, &scope)?);
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
        Commands::Add => {
            let mut result = Vec::new();
            for scope in &common_selections {
                result.extend(add_remote(&cli.project, scope, &common_provider)?);
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
        Commands::Remove => {
            let mut result = Vec::new();
            for scope in &common_selections {
                result.extend(remove(&cli.project, scope)?);
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
        Commands::Forget => {
            let mut result = Vec::new();
            for scope in &common_selections {
                result.extend(forget(&cli.project, scope)?);
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
        Commands::Fetch => {
            let mut result = Vec::new();
            for scope in &common_selections {
                result.extend(fetch(&cli.project, scope, &common_provider)?);
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
        Commands::Status { check } => {
            let mut result = Vec::new();
            for scope in &common_selections {
                result.extend(compare(&cli.project, scope)?);
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
        Commands::Diff { exit_code } => {
            let mut result = Vec::new();
            for scope in &common_selections {
                result.extend(diff(&cli.project, scope)?);
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
        Commands::Pull { yes, missing } => {
            if !yes && !io::stdin().is_terminal() {
                bail!("--yes is required to apply Pull non-interactively");
            }
            if !yes && !confirm("Apply observed changes to desired Resources")? {
                bail!("Pull cancelled");
            }
            let mut result = Vec::new();
            for scope in &common_selections {
                result.extend(pull(&cli.project, scope, missing.map(Into::into))?);
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
            dry_run,
            uncommitted,
            untracked,
            missing,
            yes,
            new_plan,
        } => {
            let interactive = !cli.non_interactive && io::stdin().is_terminal();
            let mut confirmed = yes;
            if interactive && !confirmed {
                let mut required = false;
                for scope in &common_selections {
                    required |= push_confirmation_required(
                        &cli.project,
                        scope,
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
            for scope in &common_selections {
                result.extend(push(
                    &cli.project,
                    scope,
                    dry_run,
                    uncommitted.map(Into::into),
                    untracked.map(Into::into),
                    interactive,
                    confirmed,
                    new_plan,
                    missing.map(Into::into),
                    &common_provider,
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

fn selections(cli: &Cli) -> Result<Vec<Selection>> {
    let environments = if cli.all_environments {
        load_project(&cli.project)?
            .environments
            .keys()
            .cloned()
            .map(Some)
            .collect()
    } else if cli.environment.is_empty() {
        vec![None]
    } else {
        cli.environment.iter().cloned().map(Some).collect()
    };
    Ok(environments
        .into_iter()
        .map(|environment| Selection {
            environment,
            targets: cli.target.clone(),
            namespaces: cli.namespace.clone(),
            types: cli.resource_type.clone(),
            ids: cli.id.clone(),
        })
        .collect())
}
fn one_environment_names(environments: &[String], all: bool) -> Result<Option<String>> {
    if all || environments.len() > 1 {
        bail!("this command accepts exactly one Environment");
    }
    Ok(environments.first().cloned())
}
fn provider_map(cli: &Cli) -> BTreeMap<String, String> {
    cli.provider_values.iter().cloned().collect()
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
    match format {
        OutputFormat::Yaml => print!("{}", serde_yaml::to_string(value)?),
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(value)?),
    }
    Ok(())
}
