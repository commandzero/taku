## Why

Taku's repeatable global Resource selectors form a shallow query language that appears on commands which ignore it, obscures the Target-to-Application hierarchy, and gives the shell too little structure for useful completion. Before a stable release, the CLI should adopt one consistent rule: positional arguments identify domain objects in dependency order, while command-specific options modify scope, policy, or execution.

## What Changes

- **BREAKING**: Introduce one hierarchical Resource Path, `<target> <resource-type> <id>...`, and remove global `--target`, `--type`, and `--id` selectors without compatibility aliases.
- **BREAKING**: Require the complete Resource Path with one or more IDs for `add`, `remove`, and `forget`; all IDs in one invocation share the selected Environment, Target, Resource Type, and optional Namespace.
- **BREAKING**: Let `list`, `fetch`, `status`, `diff`, `pull`, and `push` accept an optional Resource Path prefix: no path selects the command's broad default scope, Target narrows to one Target, Resource Type further narrows it, and IDs select exact Resources.
- **BREAKING**: Make remote `list` require Target and Resource Type so remote discovery is Application-aware and a namespaced Type can require an explicit Namespace.
- **BREAKING**: Make Resource scope options command-specific. A Resource Path selects exactly one Environment; broad multi-Environment execution remains available only where the command already supports it and no Resource Path is present.
- **BREAKING**: Separate Application and Target terminology by moving `app add` and `app rename` to `target add` and `target rename`; `taku target` lists Targets in the selected Environment while `taku app` continues to list Applications and own Application Source refresh.
- Preserve positional Application names for `install` and `update`, Environment names for `context set`, role-specific Promotion options, and existing execution-policy options.
- Add installable shell integration and context-aware completion across the entire grammar: Environments, Applications, Targets, Resource Types, Namespaces, Resource IDs, Promotion mappings, fixed values, and filesystem paths.
- Make dynamic completion command-aware, prefix-filtered, deterministic, non-interactive, and read-only; remote candidates may perform authenticated version or List reads but never create Baselines, caches, or desired state.
- Do not provide backwards compatibility or migration aliases for the pre-release command grammar.

## Capabilities

### New Capabilities

- `cli-command-grammar`: Defines the unified command hierarchy, Resource Path semantics, command-specific scope, Target management, and breaking syntax for all Taku commands.
- `shell-completion`: Defines installable shell integration and command-aware static and dynamic completion throughout the unified grammar.

### Modified Capabilities

None. This repository does not yet contain main OpenSpec capabilities.

## Impact

- Affects the full Clap grammar and dispatch in `src/main.rs`, Resource selection and lifecycle interfaces in `resource-control`, command help, every CLI integration test family, ignored live tests, `README.md`, and Command Scope/Target terminology in `CONTEXT.md`.
- Replaces broad selector vectors at the CLI seam with a shared hierarchical Resource Path and command policies that translate it into engine selections.
- Adds a Target listing command and moves Target addition/rename without changing Project file formats or Target behavior.
- Requires a completion-script entry point plus a deep, read-only candidate module spanning Project context, Application sources, effective Resource Types, local inventory, Deletion Markers, provider keys, and remote listing.
- Requires a Clap-compatible completion dependency or equivalent shell integration for Bash, Zsh, Fish, Elvish, and PowerShell.
