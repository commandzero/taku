# Guide

1. [Application definitions](application-definitions.md) - Versioned catalogs, operation mappings, projections, and Git sources.
2. [Live API validation](live-api-validation.md) - Explicit opt-in API validation, retained artifacts, and legacy test hazards.
3. [Using Taku](usage.md) - Resource workflows, project configuration, output, completion, and safety.

# Subdirectories

1. [adr](adr/index.md) - Contains 29 entries: Separate resource-control from Taku, Separate observation, adoption, and Git history, Allow dependency-aware partial execution, Use guarded deletion markers for partial inventories, Store complete Resources per Environment, Separate Environments from Targets, Gate writes on Resource Type Definition changes, Default to Unguarded concurrency, Reject unknown configuration, Keep environment providers isolated, Make Push the only remote mutator, Use three-way comparison for Pull, Treat unexplained remote absence as a Conflict, Confine Resource files to validated Resource trees, Require a Git worktree root and explicit layout, Vendor Resource Type Catalogs per Project, Use Explicit Application Source Refresh, Configure Push Git-State Policies Independently, Journal Partial Push Execution, Default Command Output to Structured YAML, Default Commands to the Current Environment, Wrap Resolved Credentials as Secrets, Drop Sensitive Fields Before Persistence, Scope Target Names to Environments, Use Destination-Owned Promotion Mappings, Keep Remote Observation Explicit, Model optional Resource namespaces explicitly, Keep Resource Metadata Policy Directory-Local, Separate canonical Resources from operation collections.
2. [agents](agents/index.md) - Contains 3 entries: Domain Docs, Issue tracker: GitHub, Triage Labels.
3. [repository](repository/index.md) - Contains 3 entries: Documentation bundle, Contributing, Releases and compatibility.
