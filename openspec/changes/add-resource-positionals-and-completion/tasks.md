## 1. Unified Resource Path Module

- [ ] 1.1 Add validated Resource Path and Resource Scope types representing only Target, Target/Type, and Target/Type/non-empty-IDs hierarchy states plus optional Namespace and Environment context.
- [ ] 1.2 Add Exact, Partial, and Remote-List scope policies that enforce path completeness, Environment cardinality, Namespace placement, and namespaced-Type rules before lifecycle dispatch.
- [ ] 1.3 Convert validated Resource Scopes into existing engine `Selection` values, preserving broad multi-Environment expansion only when a Partial command has no Resource Path.
- [ ] 1.4 Test the Resource Path interface for every valid prefix, variadic IDs, hierarchy gaps, exact-path requirements, remote-list requirements, Namespace behavior, and path/multi-Environment conflicts.

## 2. CLI Grammar and Resource Commands

- [ ] 2.1 Restrict root-global arguments to Project path, output format, and non-interactive mode; add reusable command-specific Environment, Namespace, provider, and lifecycle-policy argument groups.
- [ ] 2.2 Change Add, Remove, and Forget to require `<target> <resource-type> <id>...`, route them through Exact scope policy, and emit one existing lifecycle result per matched ID.
- [ ] 2.3 Change local List, Fetch, Status, Diff, Pull, and Push to accept the optional hierarchical Resource Path and route it through Partial scope policy.
- [ ] 2.4 Make remote List require Target and Resource Type with optional IDs, reject local `--untracked`, and route remote scope through Remote-List policy.
- [ ] 2.5 Remove `--target`, `--type`, and `--id` from the parser entirely and ensure irrelevant Environment, Namespace, provider, and lifecycle options are absent from unrelated command help.
- [ ] 2.6 Add parser and CLI integration tests for every Resource command's no-path, partial-path, exact-path, variadic-ID, Namespace, Environment, option-conflict, and removed-syntax behavior.

## 3. Application, Target, and Remaining Commands

- [ ] 3.1 Add a read-only Target listing operation that resolves one Environment and reports each Target with its Application identity.
- [ ] 3.2 Add `taku target` listing plus `target add <application> [name] --url <url>` and `target rename <old> <new>` using the existing Target Addition and Rename implementations.
- [ ] 3.3 Remove `app add` and `app rename` while preserving `taku app` Application listing and `app refresh` Application Source behavior.
- [ ] 3.4 Preserve and verify positional Application behavior for Install/Update, Context Environment selection, initialization options, Project-wide Validate, and role-specific Promotion options under the reduced global grammar.
- [ ] 3.5 Add command help and integration tests for Application/Target separation, exact Environment requirements, legacy-command rejection, and unchanged non-Resource execution semantics.

## 4. Read-Only Completion Candidates

- [ ] 4.1 Extract effective Target discovery and exact remote Type listing into read-only queries, leaving Baseline persistence in explicit operational List/Add wrappers.
- [ ] 4.2 Add one candidate query/result interface covering Environment, Application, Target, Promotion, provider-key, Namespace, and command-specific Resource Path intents.
- [ ] 4.3 Implement local candidates from Project metadata, embedded/current Application Source cache, installed Applications, Canonical inventory, and Deletion Markers, including intent filtering and `default` Namespace.
- [ ] 4.4 Implement remote Add/List Type and ID candidates through one selected Target using read-only version and bounded Many List behavior with existing non-interactive provider precedence.
- [ ] 4.5 Implement exact-value descriptions, prefix filtering, lexical sorting, deduplication, and exclusion of already-entered repeatable Application or Resource ID values.
- [ ] 4.6 Test the candidate interface across multiple Environments and Applications, Target management, Promotion mappings, version-qualified Types, local lifecycle intents, remote tracked/untracked IDs, Namespaces, provider keys without values, and deterministic filtering.
- [ ] 4.7 Assert candidate success and failure never refresh Application Sources or write Baselines, caches, desired state, Deletion Markers, or other Project metadata.

## 5. Shell Integration

- [ ] 5.1 Add the Clap-compatible completion dependency and `completion <shell>` for Bash, Zsh, Fish, Elvish, and PowerShell with raw sourceable output.
- [ ] 5.2 Connect parser metadata to static subcommand, command-option, positional, fixed-value, conflict, and filesystem-path completion.
- [ ] 5.3 Connect the runtime completion protocol or thin hidden adapter to the candidate module while preserving partial Project, command, Environment, Resource Path, Namespace, modifier, and already-entered value context.
- [ ] 5.4 Implement interactive error handling that converts dynamic candidate failures into silent empty results without changing normal command or script-generation diagnostics.
- [ ] 5.5 Add script-generation smoke tests for all supported shells and runtime tests covering every dynamic argument family, irrelevant-option omission, exact insertion, repeatable exclusion, and silent failures.

## 6. Repository Migration and Verification

- [ ] 6.1 Rewrite every Resource command invocation in unit, CLI, scheduler, Application, pagination, versioning, and ignored live tests to the unified Resource Path grammar, retaining only intentional negative tests for removed syntax.
- [ ] 6.2 Update `README.md` command examples, Target management, broad versus path-scoped workflows, Namespace rules, remote List requirements, and completion installation/read-only-remote behavior.
- [ ] 6.3 Update `CONTEXT.md` Command Scope, Target Addition, and Target Rename definitions to the unified Resource Path and `taku target` terminology.
- [ ] 6.4 Run formatting, lints, the complete non-ignored test suite, focused CLI help/completion checks, and strict OpenSpec validation; fix all failures and confirm no Project schema or data migration was introduced.
