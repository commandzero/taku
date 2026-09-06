## Context

See `proposal.md` for motivation and the delta specs for the behavioral contract.

The root parser currently owns repeatable global Environment, Target, Namespace, Resource Type, ID, and provider fields. It eagerly converts them into one broad `Selection` per Environment before dispatch, even for commands that ignore Resource selection. The same flat representation serves exact lifecycle actions, broad comparison/reconciliation, Application management, Target management, and remote listing despite their different invariants.

The engine's `Selection` remains useful for iterating a resolved set, but it is a poor CLI interface: vector fields admit gaps and disjoint combinations, do not express the Target-to-Application relationship, and force every completion callback to rediscover command intent. The change needs a deep Resource Path module at the CLI-to-engine seam and a corresponding read-only candidate module.

Target completion is local, but effective remote Resource Types may require version discovery and Add/remote-List IDs require authenticated Many List Operations. Existing `remote_list` also creates a Baseline when absent, which is valid during operational listing but forbidden while completing a command.

## Goals / Non-Goals

**Goals:**

- Make invalid Resource Path states unrepresentable after CLI preflight.
- Centralize Environment, Namespace, and per-command scope rules rather than scattering validation across match arms.
- Preserve the engine's reusable set-oriented lifecycle behavior behind a smaller CLI interface.
- Derive static completion from the parser and dynamic completion from one command-aware candidate module.
- Test Resource Path and completion behavior through the same interfaces used by dispatch and shell adapters.

**Non-Goals:**

- Changing Project, Application, Target, Resource, Baseline, cache, or Deletion Marker file formats.
- Adding fuzzy search, interactive selection, a completion daemon, automatic Application Source refresh, or persistent completion caches.
- Making Namespace positional; its conditional presence would make positional arity depend on a remotely resolved Type.
- Replacing role-specific Promotion options with ambiguous source/destination positionals.
- Retaining the pre-release selector or `app add`/`app rename` syntax.

## Decisions

### 1. Parse every Resource command through one Resource Path module

Introduce a validated Resource Path enum with only the states commands can use: Target, Target plus Resource Type, or Target plus Resource Type and a non-empty list of IDs. Absence of a path remains outside the enum. Exact parser arguments for Add/Remove/Forget and optional parser arguments for List/Fetch/Status/Diff/Pull/Push both convert through this interface.

A Resource Scope value combines an optional validated Resource Path, Environment selection, and optional Namespace. One policy enum defines the accepted shape for a command:

- `Exact`: one Environment and a complete path with IDs.
- `Partial`: zero or one Environment when a path is present; existing broad Environment expansion when absent.
- `RemoteList`: one Environment plus Target and Resource Type, with IDs optional.

The Resource Path module validates hierarchy, Environment cardinality, and Namespace placement before converting the result into the engine's existing `Selection` values. Command match arms choose a policy and lifecycle operation but do not rebuild validation rules.

The engine validates namespaced-Type rules after resolving the effective Resource Type. Local inventory rejects Namespace on non-namespaced Types and requires it for exact namespaced IDs, while allowing Type-only selection across local Namespaces. Remote queries require Namespace for namespaced listing. These checks belong beside Type resolution because remote discovery can select a version-specific definition unavailable to CLI preflight. CLI integration tests exercise these production checks; Resource Scope tests cover structural placement and Selection conversion only.

This is a deep module: deleting it would redistribute parsing invariants, Environment resolution, Namespace validation, and Selection construction across nine command arms and completion callbacks.

**Alternative considered:** Give every command independent positional fields and construct `Selection` directly. Rejected because superficially consistent help would hide duplicated, drifting semantics.

**Alternative considered:** Replace engine `Selection` everywhere with Resource Path. Rejected because the engine legitimately supports broad set iteration and Environment expansion; Resource Path is the human interface, not a replacement for internal selection.

### 2. Keep batching only inside one complete parent path

IDs are variadic after Target and Resource Type. Add, Remove, and Forget require at least one; partial commands accept none or many. All IDs share one Environment, Target, Type, and Namespace, preventing disjoint selector expressions while retaining the common batch workflow.

Lifecycle functions may continue returning vectors and using their existing set-oriented implementations. Add's existing exact-ID requirement remains, but its interface no longer needs to pretend Target or Type can be omitted by a CLI caller. Each command produces one result per matched ID in the existing output envelope.

**Alternative considered:** Restrict exact commands to one ID. Rejected because it removes useful batching without improving hierarchy or completion; subsequent completion can safely exclude already-entered IDs.

**Alternative considered:** Retain repeatable selector options for advanced batches. Rejected because two equivalent syntaxes undermine help, validation, and completion. Users run separate commands for disjoint parent paths.

### 3. Make scope options local to commands

Keep only truly cross-cutting parser fields global: Project path, output format, and non-interactive execution. Flatten reusable command-specific argument groups where applicable:

- Environment arguments on Target management and Resource commands.
- Broad multi-Environment selection only on partial Resource commands.
- Namespace on Resource commands.
- Provider values only on commands that may contact Targets.
- Existing policy arguments only on the lifecycle command that consumes them.

This ensures Clap help and static completion advertise only meaningful options. Resource Path validation rejects multi-Environment scope as soon as any path segment is present.

**Alternative considered:** Leave all fields global and reject irrelevant combinations after parsing. Rejected because ignored-looking options would remain in help and completion, preserving the core UX defect.

### 4. Separate Application and Target command ownership

Keep `app` as the Application listing command with `app refresh` for Application Source refresh. Add a `target` command that lists the selected Environment's Targets and owns the existing Add and Rename implementations. Target listing is a read-only projection of Project metadata and includes each Target's Application identity.

Installation and update remain concise top-level commands because they already take positional Application identities and their names clearly describe definition lifecycle. Context and Promotion retain their current primary grammar; only their dynamic values gain completion.

**Alternative considered:** Move install/update beneath `app`. Rejected because extra nesting does not improve identity ordering or eliminate ambiguity, and those top-level verbs already operate exclusively on Applications.

### 5. Add one deep command-aware candidate module

Expose one candidate function whose query enum carries completion intent, the effective Project and command context, the partial Resource Path when applicable, the token prefix, and values already selected. Intents cover Environments, install/update/Target-add Applications, Target management, Promotion endpoints, provider keys, and each Resource command's Target/Type/ID policy. The result contains exact candidate values with optional descriptions.

The module owns candidate source rules:

- Environment and Target candidates come from Project metadata.
- Install candidates come from current embedded/Git-cache listings minus installed and already selected Applications; Update candidates come from installed Applications.
- Target-add Applications come from available Application listings; Target-rename and Promotion Target candidates use the appropriate Environment.
- Remote Resource intents resolve the Target's installed Application, perform read-only effective-version discovery, and use listable Resource Types or remote entries.
- Local Resource intents use Canonical inventory and Deletion Markers according to lifecycle behavior without contacting a Target.
- Namespace candidates are `default` plus relevant locally known Namespace values; completion never probes every possible remote Namespace.
- Provider completion returns field names ending in `=` and never values.
- The module prefix-filters, deduplicates, excludes already-entered repeatable identities, and lexically sorts candidates.

The module returns errors through its interface. The interactive shell adapter, not the module, owns the silent-empty failure policy. No public transport port is introduced solely for completion; existing local test Projects and HTTP test servers provide two real implementations at internal filesystem/transport seams.

Namespace and exact-ID intents resolve the selected Resource Type before returning candidates: non-namespaced Types do not receive Namespace suggestions, and namespaced Types do not receive ID suggestions until a Namespace is present. Remote Resource Type candidates require a Many List Operation rather than merely the presence of any List Operation.

**Alternative considered:** Add independent Clap callbacks for each argument. Rejected because source policy, side-effect rules, filtering, and error handling would spread across shallow adapters.

### 6. Separate read-only remote queries from operational persistence

Extract effective Target discovery and exact remote Type listing into an internal read-only query that accepts resolved context and never writes a Baseline or cache. Operational remote List and Add may explicitly persist a missing Baseline around that query where existing behavior requires it. Completion invokes only the read-only query.

Do not implement this as `persist_baseline: bool`; such an interface makes callers understand an internal side-effect switch and is easy to misuse. The read-only query returns data, while the operational wrapper owns persistence.

Tests at the candidate-module interface assert that version and ID completion leave Baseline, cache, Application Source, and desired-state paths unchanged on both success and failure.

**Alternative considered:** Call operational `remote_list` and delete artifacts afterward. Rejected because cleanup is racy, failure-prone, and not read-only.

### 7. Generate shell integration from Clap and use a thin runtime adapter

Add `completion <shell>` using a Clap-compatible completion dependency for Bash, Zsh, Fish, Elvish, and PowerShell. Static grammar comes from the same command factory as parsing. Dynamic positions route the current token stream and cursor context to the candidate module through the dependency's runtime protocol when sufficient; otherwise a hidden machine-oriented entry point acts only as a shell adapter.

Completion script and runtime candidate output bypass schema envelopes. Runtime completion dispatch occurs before normal command execution, never prompts, and converts candidate errors to successful empty dynamic output without printing diagnostics. Normal commands and script generation retain ordinary errors.

The completion command factory may hide conditionally invalid options from the generated parser view, such as repeated Environment selection after a Resource Path, while retaining the same underlying Clap grammar. Its token adapter interprets option arity according to the active command when option names have different meanings across commands.

**Alternative considered:** Hand-maintain five complete shell scripts. Rejected because duplicating grammar would drift; shell-specific code should only adapt one parser and candidate interface.

### 8. Test through the two new interfaces

Parser/dispatch tests exercise Resource Path conversion and command policies, while candidate tests invoke the candidate module with temporary Projects and existing HTTP test servers. Tests assert observable scopes, candidates, output, failures, and filesystem side effects rather than internal helper calls. Old tests whose only purpose was exercising flat selector construction are replaced by interface-level Resource Path tests.

Shell tests smoke-test generated scripts for all supported shells and exercise the runtime adapter with token streams representing every dynamic argument family. The complete CLI integration suite remains the final proof that the converted `Selection` preserves lifecycle behavior.

### 9. Group root help without nesting the grammar

Keep all commands as direct children of `taku`. Customize only the root help template so it renders two command sections: `taku configuration` and `Resource management`. Subcommand help, parsing, dispatch, and completion continue to use the same Clap command tree.

The grouped listing is covered by an integration test that also verifies representative Resource commands remain flat. This makes scope visible during discovery without adding keystrokes or weakening positional completion.

**Alternative considered:** Add a `resource` command namespace. Rejected because the requested distinction is navigational help, not a new grammar level, and nesting would add typing to every Resource operation.

## Risks / Trade-offs

- **[Users lose arbitrary disjoint selection in one invocation]** → Variadic IDs preserve batching beneath one parent path; broad no-path execution remains; separate invocations make disjoint intent explicit.
- **[Remote completion can be slow]** → Target and all local lifecycle completion remain local; only effective remote Type and ID intents contact one selected Target using existing bounded version/List behavior.
- **[A local Namespace list may not include a valid new remote Namespace]** → Always offer `default`, permit free-form input, and avoid unsafe or expensive speculative cross-Namespace listing.
- **[Moving formerly global options changes option placement]** → This is an intentional pre-stable break; regenerate help/completion and update every example atomically with no alias layer.
- **[Completion and execution could select different data]** → Both use the same Resource Path, Environment, Application resolution, Namespace, inventory, Deletion Marker, and read-only remote query implementations.
- **[Silent completion failure is difficult to diagnose]** → Candidate errors remain observable at the module interface and in tests; normal list/lifecycle commands expose full diagnostics.
- **[Shell runtime protocols differ]** → Keep one candidate module and add adapter smoke tests for all five shells, with end-to-end runtime cases for the chosen dynamic protocol.

## Migration Plan

1. Land the Resource Path module, parser reorganization, Target command split, candidate module, shell integration, documentation, and converted tests atomically.
2. Rewrite all repository and ignored live-test commands to the unified positional grammar; retain only negative tests for removed syntax.
3. Update `CONTEXT.md` Command Scope, Target Addition, and Target Rename terminology alongside `README.md` examples and completion installation instructions.
4. Provide no data migration, deprecation, or compatibility aliases because Project formats are unchanged and no stable CLI has shipped.
5. Users regenerate completion scripts after upgrading. Rollback is a code rollback plus regeneration/removal of scripts produced by the newer binary.
