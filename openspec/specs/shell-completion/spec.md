# shell-completion Specification

## Purpose

Provides installable, command-aware shell completion for constructing valid Taku commands from Project, Application, Target, and Resource context.

## Requirements

### Requirement: Taku emits installable shell integration
The system SHALL provide `taku completion <shell>` for Bash, Zsh, Fish, Elvish, and PowerShell, and SHALL write the selected shell's integration directly to standard output without a Taku YAML or JSON result envelope.

#### Scenario: Generate Zsh integration
- **WHEN** the user runs `taku completion zsh`
- **THEN** standard output contains sourceable Zsh completion integration and the command exits successfully

#### Scenario: Unsupported shell is requested
- **WHEN** the user requests a shell outside the supported set
- **THEN** command parsing fails with the supported shell values and emits no partial script

### Requirement: Completion reflects the current CLI grammar
Installed completion SHALL derive subcommands, command-specific options, positional structure, option conflicts, and fixed option values from the same grammar used for command parsing.

#### Scenario: Complete root commands
- **WHEN** the cursor is at the first incomplete token after `taku`
- **THEN** completion offers matching commands including `app`, `target`, Resource lifecycle commands, and `completion`

#### Scenario: Irrelevant option is omitted
- **WHEN** the user completes options for `taku app`
- **THEN** completion does not offer Resource Path scope options that App does not accept

#### Scenario: Complete a fixed value
- **WHEN** the cursor is at an output, layout, policy, missing-state, or shell value
- **THEN** completion offers the matching values declared by that command

### Requirement: Environment completion honors command context
The system SHALL complete Environment names from Project metadata for `context set`, command-specific `--environment`, and Promotion `--from` and `--to`. It SHALL suppress multi-Environment options when the partially typed command contains a Resource Path or otherwise requires exactly one Environment.

#### Scenario: Complete Context Environment
- **WHEN** the user completes `taku context set pr`
- **THEN** completion offers matching Environment names such as `prod`

#### Scenario: Resource Path already exists
- **WHEN** an exact or partial Resource Path is present
- **THEN** completion does not offer `--all-environments` or a second `--environment`

### Requirement: Application and Target completion is purpose-aware
The system SHALL offer available uninstalled Applications for `install`, installed Applications for `update`, available Applications for `target add`, current-Environment Targets for `target rename`, and source/destination Environment Targets for Promotion options. It SHALL exclude duplicate positional Application values already entered.

#### Scenario: Complete an Application to install
- **WHEN** Elasticsearch is available and uninstalled while Kibana is already installed
- **THEN** `taku install <TAB>` offers Elasticsearch and does not offer Kibana

#### Scenario: Complete a Target to rename
- **WHEN** the selected Environment contains Targets `es` and `kb`
- **THEN** `taku target rename <TAB>` offers `es` and `kb`

#### Scenario: Complete a destination Promotion Target
- **WHEN** the destination Environment is known from `--to` or current context
- **THEN** `--to-target` completion offers only Targets from that destination Environment

### Requirement: Resource Path Target completion uses one Environment
For the first Resource Path positional, the system SHALL offer Target names from the one current or explicitly selected Environment and SHALL honor the effective `--project` argument.

#### Scenario: Complete Targets in a multi-Application Environment
- **WHEN** Environment `dev` defines Targets `es` and `kb` for different Applications and the user completes `taku status `
- **THEN** completion offers both `es` and `kb`

#### Scenario: Explicit Environment changes candidates
- **WHEN** the command contains `--environment prod`
- **THEN** Resource Path completion offers Targets from `prod` and omits Targets found only in other Environments

### Requirement: Resource Type completion is Target- and command-aware
For the second Resource Path positional, the system SHALL resolve the selected Target's Application. Add and remote List SHALL offer effective Resource Types supporting remote Many List; local lifecycle commands SHALL offer Types relevant to the selected Target's managed inventory or Deletion Markers for that command.

#### Scenario: Complete a remotely adoptable Type
- **WHEN** Target `es` uses Elasticsearch and the user completes `taku add es `
- **THEN** completion offers matching listable Elasticsearch Resource Types and omits Kibana-only or non-listable Types

#### Scenario: Complete a local Status Type
- **WHEN** only `ingest_pipelines` and `roles` have managed state on Target `es`
- **THEN** `taku status es <TAB>` offers those relevant Types without contacting the Target

#### Scenario: Target is unknown
- **WHEN** the first positional is not a Target in the selected Environment
- **THEN** completion returns no dynamic Resource Type candidates

### Requirement: Namespace completion uses known exact values
For a namespaced Resource Type, the system SHALL offer the literal `default` Namespace and distinct matching Namespace names already present in relevant local inventory or Deletion Markers. It SHALL insert the exact Namespace value and SHALL NOT perform speculative cross-Namespace remote listing.

#### Scenario: Complete known Namespaces
- **WHEN** Target `kb` has managed `saved_objects` in `default` and `engineering`
- **THEN** `--namespace` completion for that Target and Type offers `default` and `engineering`

#### Scenario: Namespace parent path is incomplete
- **WHEN** Target and Resource Type are not both present
- **THEN** completion returns no dynamic Namespace candidates

### Requirement: Resource ID completion follows lifecycle intent
For Resource ID positionals, the system SHALL offer only candidates relevant to the command: untracked remote IDs for Add; remote IDs matching List modifiers for remote List; managed Canonical Resource IDs for Remove and local List; managed Resource and Deletion Marker IDs for Forget; and locally selected Resources or markers applicable to Fetch, Status, Diff, Pull, and Push.

#### Scenario: Complete Add IDs
- **WHEN** remote IDs `pipe-1` and `pipe-2` exist and only `pipe-1` is managed
- **THEN** `taku add es ingest_pipelines <TAB>` offers `pipe-2` and omits `pipe-1`

#### Scenario: Complete Remove IDs
- **WHEN** IDs `pipe-1` and `pipe-2` are managed Canonical Resources
- **THEN** `taku remove es ingest_pipelines <TAB>` offers both IDs without a remote request

#### Scenario: Complete Forget IDs
- **WHEN** one matching ID is a Canonical Resource and another is a Deletion Marker
- **THEN** Forget completion offers both IDs

#### Scenario: Namespaced IDs lack Namespace
- **WHEN** a namespaced Resource Type is selected and the command requires exact IDs but has no `--namespace`
- **THEN** completion returns no Resource ID candidates and does not search across Namespaces

#### Scenario: Previously entered ID is excluded
- **WHEN** one or more IDs are already positional values in the partial command
- **THEN** completion omits those IDs from subsequent candidates

### Requirement: Provider key and filesystem completion avoid secret values
Where `--set` is valid and enough Target context exists, completion SHALL offer configured provider field names with the assignment delimiter but SHALL NOT suggest, read, or expose credential values. Project and Promotion Project path options SHALL use filesystem path completion.

#### Scenario: Complete a provider key
- **WHEN** the selected Target accepts an `authorization` provider field and the user completes `--set auth`
- **THEN** completion may insert `authorization=` and does not include its value

#### Scenario: Complete a Project path
- **WHEN** the user completes `--project`, `--from-project`, or `--to-project`
- **THEN** shell filesystem candidates are offered

### Requirement: Dynamic candidates are deterministic and preserve exact identities
The system SHALL prefix-filter dynamic candidates against the token being completed, remove duplicates, sort them lexically, and insert exact domain identities. Human-readable descriptions MAY accompany candidates but SHALL NOT replace inserted values.

#### Scenario: Prefix filters candidates
- **WHEN** available IDs are `alpha`, `beta`, and `bravo` and the current token is `b`
- **THEN** completion offers `beta` and `bravo` in lexical order

#### Scenario: Resource display name differs from ID
- **WHEN** a Resource ID differs from its display name
- **THEN** selecting its completion inserts the exact Resource ID

### Requirement: Completion discovery is non-interactive and read-only
Completion SHALL NOT prompt, write Project metadata, refresh an Application Source, install or update Applications, create or update Baselines or caches, change desired state, or invoke remote mutation Operations. Remote candidates SHALL use only read-only version and List Operations with existing non-interactive credential resolution.

#### Scenario: Completion runs without a Baseline
- **WHEN** completion discovers an effective Resource Type or remote ID for a Target without a Baseline
- **THEN** the Baseline path remains absent afterward

#### Scenario: Application Source cache is stale or absent
- **WHEN** Application completion has no refreshed Git Source cache
- **THEN** it uses only currently available local/embedded information and does not refresh the source

#### Scenario: Credentials are unavailable
- **WHEN** remote completion cannot resolve authentication non-interactively
- **THEN** it returns no affected dynamic candidates without prompting or exposing credential material

### Requirement: Dynamic completion failure does not disrupt the shell
When Project context, installed Application data, version discovery, authentication, local inventory, or a remote read is unavailable, the interactive adapter SHALL return no affected dynamic candidates and SHALL NOT emit diagnostics into the command line. Static grammar candidates SHALL remain available where applicable.

#### Scenario: Target is unreachable during ID completion
- **WHEN** the remote Target cannot be reached while completing Add or remote List IDs
- **THEN** completion leaves the partially typed command unchanged, emits no Resource ID candidates, and prints no error into the prompt
