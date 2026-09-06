# cli-command-grammar Specification

## Purpose

Defines one predictable Taku command grammar in which positional domain identities form a hierarchy and command-specific options modify behavior or scope.

## Requirements

### Requirement: Primary domain identities are positional
The system SHALL represent the primary objects acted on by a command as positional arguments in dependency order. It SHALL reserve options for execution policy, alternate context, output, or other modifiers and SHALL reject scope options on commands that do not use them.

#### Scenario: Irrelevant Resource option is rejected
- **WHEN** the user supplies `--target`, `--type`, `--id`, or `--namespace` to an Application, Context, Promotion, initialization, installation, update, or validation command
- **THEN** command parsing fails instead of silently accepting an ignored option

#### Scenario: Execution policy remains an option
- **WHEN** the user configures Push dry-run, Git-state, missing-state, confirmation, or plan-replacement behavior
- **THEN** those modifiers remain named options rather than positional identities

### Requirement: Resource commands share one hierarchical Resource Path
The system SHALL define a Resource Path as `<target> <resource-type> <id>...`. A command that accepts a partial path SHALL accept only a contiguous prefix: no path, Target only, Target and Resource Type, or Target and Resource Type followed by one or more IDs.

#### Scenario: Target-only scope
- **WHEN** the user runs `taku status es`
- **THEN** Status is limited to managed Resources and Deletion Markers belonging to Target `es`

#### Scenario: Target and Resource Type scope
- **WHEN** the user runs `taku fetch es ingest_pipelines`
- **THEN** Fetch is limited to managed `ingest_pipelines` Resources on Target `es`

#### Scenario: Multiple exact IDs share their parent path
- **WHEN** the user runs `taku diff es ingest_pipelines pipe-1 pipe-2`
- **THEN** Diff is limited to IDs `pipe-1` and `pipe-2` within that one Target and Resource Type

#### Scenario: Hierarchy segment is missing
- **WHEN** a user attempts to select a Resource Type without a Target or an ID without both a Target and Resource Type
- **THEN** command parsing fails instead of searching unrelated Targets or Applications

### Requirement: Exact lifecycle commands require a complete Resource Path
`add`, `remove`, and `forget` SHALL require one Target, one Resource Type, and one or more Resource IDs. All IDs in an invocation SHALL share one selected Environment and optional Namespace.

#### Scenario: Adopt multiple Resources
- **WHEN** the user runs `taku add es ingest_pipelines pipe-1 pipe-2`
- **THEN** Add attempts to adopt only the two untracked remote Resources addressed by those IDs from that Target and Resource Type

#### Scenario: Mark multiple Resources for deletion
- **WHEN** the user runs `taku remove es ingest_pipelines pipe-1 pipe-2`
- **THEN** Remove attempts to replace only those two managed Resources with guarded Deletion Markers

#### Scenario: Forget Resources or markers
- **WHEN** the user runs `taku forget es ingest_pipelines pipe-1 pipe-2`
- **THEN** Forget attempts to stop managing only matching Canonical Resources or Deletion Markers

#### Scenario: Exact lifecycle path is incomplete
- **WHEN** Add, Remove, or Forget is invoked without Target, Resource Type, or at least one ID
- **THEN** command parsing fails before Project or remote state changes

### Requirement: Selection lifecycle commands accept an optional Resource Path
Local `list`, `fetch`, `status`, `diff`, `pull`, and `push` SHALL accept an optional Resource Path prefix. With no Resource Path they SHALL retain their existing broad default over the selected Environment or Environments; each supplied segment SHALL narrow that scope hierarchically.

#### Scenario: Broad command has no path
- **WHEN** the user runs `taku status` in one current Environment
- **THEN** Status evaluates its existing default scope across that Environment

#### Scenario: Pull selects exact Resources
- **WHEN** the user runs `taku pull es ingest_pipelines pipe-1 pipe-2 --yes`
- **THEN** Pull accepts observed changes only for those addressed Resources

#### Scenario: Push selects a Resource Type
- **WHEN** the user runs `taku push es ingest_pipelines`
- **THEN** Push reconciles the selected Type's managed Resources and Deletion Markers only on Target `es`

### Requirement: Remote listing requires an Application-aware path
`taku list --remote` SHALL require a Target and Resource Type, SHALL accept zero or more IDs after them, and SHALL use `--untracked` only as a modifier of remote results.

#### Scenario: List one remote Resource Type
- **WHEN** the user runs `taku list --remote es ingest_pipelines`
- **THEN** the system lists remote `ingest_pipelines` Resources only through Target `es`

#### Scenario: List exact remote IDs
- **WHEN** the user runs `taku list --remote es ingest_pipelines pipe-1 pipe-2`
- **THEN** the system returns only remote matches for those IDs

#### Scenario: Remote path is incomplete
- **WHEN** the user runs `taku list --remote` or supplies only a Target
- **THEN** command parsing fails with usage requiring Target and Resource Type before any remote request

#### Scenario: Untracked is used with local listing
- **WHEN** the user supplies `--untracked` without `--remote`
- **THEN** command parsing fails

### Requirement: Resource Path resolves exactly one Environment
A command containing any Resource Path segment SHALL resolve exactly one current or explicit Environment and SHALL reject repeated `--environment` or `--all-environments`. Selection lifecycle commands with no Resource Path MAY retain repeated Environment and `--all-environments` execution. Exact lifecycle and Target-management commands SHALL always resolve exactly one Environment.

#### Scenario: Resource Path uses an explicit Environment
- **WHEN** the user runs `taku status --environment prod es`
- **THEN** Target `es` is resolved only within Environment `prod`

#### Scenario: Resource Path crosses Environments
- **WHEN** the user combines a Resource Path with multiple `--environment` values or `--all-environments`
- **THEN** the command fails rather than applying one Environment-scoped path ambiguously

#### Scenario: Broad Status crosses Environments
- **WHEN** the user runs `taku status --all-environments` without a Resource Path
- **THEN** Status evaluates its existing broad scope for every Environment

### Requirement: Namespace is an explicit command-specific modifier
Resource commands SHALL accept at most one `--namespace <namespace>` only when a Target and Resource Type are present. The system SHALL reject Namespace on non-namespaced Types. It SHALL require Namespace when exact IDs or remote listing address a namespaced Type, while a local Type-only scope without Namespace SHALL include that Type's managed Resources across all known Namespaces.

#### Scenario: Add a namespaced Resource
- **WHEN** the user runs `taku add --namespace default kb saved_objects object-1`
- **THEN** Add addresses only ID `object-1` in the `default` Namespace

#### Scenario: Exact namespaced path omits Namespace
- **WHEN** a namespaced Resource Path includes IDs but omits `--namespace`
- **THEN** the command fails rather than searching or guessing a Namespace

#### Scenario: Local Type-only scope spans Namespaces
- **WHEN** the user runs `taku status kb saved_objects` without IDs or `--namespace`
- **THEN** Status includes managed `saved_objects` Resources in every locally known Namespace for that Target

#### Scenario: Namespace lacks its parent path
- **WHEN** the user supplies `--namespace` without both Target and Resource Type
- **THEN** command parsing fails

### Requirement: Legacy Resource selector flags are removed
The system SHALL reject `--target`, `--type`, and `--id` on every command and SHALL NOT provide compatibility aliases, automatic translation, or warning-only behavior for the pre-release selector grammar.

#### Scenario: Legacy Add syntax is used
- **WHEN** the user runs `taku add --target es --type ingest_pipelines --id pipe-1`
- **THEN** parsing fails with usage for `taku add <target> <resource-type> <id>...`

#### Scenario: Legacy selector is mixed with a Resource Path
- **WHEN** the user supplies any removed selector alongside positional Resource Path values
- **THEN** parsing fails rather than choosing one representation

### Requirement: Application and Target commands use domain terminology
`taku app` SHALL list available and installed Applications and SHALL retain `app refresh` for Application Source refresh. `taku target` SHALL list Targets in one selected Environment and SHALL own `target add <application> [name] --url <url>` and `target rename <old> <new>`.

#### Scenario: List Applications
- **WHEN** the user runs `taku app`
- **THEN** the system returns the Project's known Application listings without Environment-specific Target data

#### Scenario: List Targets
- **WHEN** the user runs `taku target --environment prod`
- **THEN** the system returns Targets defined in Environment `prod` and their Application identities

#### Scenario: Add a Target
- **WHEN** the user runs `taku target add elasticsearch es --url https://example.test`
- **THEN** the existing Target Addition behavior creates Target `es` for Application `elasticsearch` in one selected Environment

#### Scenario: Rename a Target
- **WHEN** the user runs `taku target rename es old-es`
- **THEN** the existing Target Rename behavior renames only that Environment's Target

#### Scenario: Legacy App Target command is used
- **WHEN** the user runs `taku app add` or `taku app rename`
- **THEN** command parsing fails without a compatibility alias

### Requirement: Non-Resource commands retain concise primary arguments
Initialization SHALL retain its layout and Environment options; `install` SHALL require one or more positional Application names; `update` SHALL accept zero or more positional Application names with none meaning all installed Applications; `context set` SHALL require one positional Environment; Promotion SHALL retain distinct named source and destination options; and `validate` SHALL remain Project-wide.

#### Scenario: Install Applications
- **WHEN** the user runs `taku install elasticsearch kibana`
- **THEN** both names are treated as Applications to install

#### Scenario: Update all Applications
- **WHEN** the user runs `taku update` without Application names
- **THEN** all installed Applications are selected using existing update behavior

#### Scenario: Select Context
- **WHEN** the user runs `taku context set prod`
- **THEN** `prod` is treated as the Environment to make current

#### Scenario: Promotion retains role clarity
- **WHEN** the user supplies Promotion Environments, Targets, or Projects
- **THEN** the source and destination remain distinguishable through `--from`, `--to`, `--from-target`, `--to-target`, `--from-project`, and `--to-project`

### Requirement: Top-level help groups commands by scope
The system SHALL keep every top-level command flat while presenting the root help listing in separate `taku configuration` and `Resource management` sections. The help-only grouping SHALL NOT add a command namespace or alter parsing, dispatch, or completion tokens.

#### Scenario: Root help separates command scopes
- **WHEN** the user runs `taku help` or `taku --help`
- **THEN** `init`, `app`, `target`, `install`, `update`, `context`, `validate`, `completion`, and `help` appear under `taku configuration`
- **AND** `list`, `add`, `remove`, `forget`, `promote`, `fetch`, `status`, `diff`, `pull`, and `push` appear under `Resource management`

#### Scenario: Grouping does not introduce a namespace
- **WHEN** the user invokes or completes a Resource command such as `taku add es ingest_pipelines pipe-1`
- **THEN** the command remains directly beneath `taku` with no intervening scope subcommand
