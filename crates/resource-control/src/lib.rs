//! Generic resource-control engine used by the `taku` reference CLI.

mod application;
mod canonical;
mod completion;
mod lifecycle;
mod model;
mod observe;
mod project;
mod projection;
mod provider;
mod reconcile;
mod resolution;
mod scheduler;
mod transport;

pub use application::{
    ApplicationListing, InstallResult, RefreshResult, UpdateResult, install_applications,
    install_applications_from, list_applications, refresh_source, update_applications,
};
pub use canonical::{InventoryEntry, Selection, list_inventory};
pub use completion::{
    CompletionCandidate, CompletionIntent, CompletionQuery, completion_candidates,
};
pub use lifecycle::{
    LifecycleResult, PromotionResult, add_remote, forget, promote, promote_projects, remove,
};
pub use model::*;
pub use observe::{
    FetchResult, RemoteEntry, RemoteResourceType, fetch, is_transformation_conflict, remote_list,
    remote_list_read_only, remote_resource_types_read_only,
};
pub use project::{
    TargetListing, add_target, git_root, initialize, list_targets, load_project, rename_target,
    save_context,
};
pub use reconcile::{Comparison, DiffEntry, PullResult, compare, diff, pull};
pub use resolution::{TargetBaseline, validate_project};
pub use scheduler::{push, push_confirmation_required};
