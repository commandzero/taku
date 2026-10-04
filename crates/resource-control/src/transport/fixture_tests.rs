//! Offline transport contracts driven entirely by fixture data and real catalogs.
//!
//! Run from the workspace: `cargo test -p resource-control --locked catalog_fixtures -- --nocapture`.
//! To reproduce with another config, copy the suite, set each `fixtures[].catalog` to
//! that ResourceTypeCatalog file (relative to the suite), and provide independent
//! input/expected JSON values. Set `TAKU_TRANSPORT_SUITE=/absolute/path/suite.yaml`
//! when running the same command. No application-specific Rust changes are needed.

use std::{collections::BTreeMap, fs, path::PathBuf};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::Value;

use super::{inbound, outbound};
use crate::ResourceTypeCatalog;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Suite {
    fixtures: Vec<Fixture>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    catalog: PathBuf,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    name: String,
    resource_type: String,
    variant: usize,
    input: Value,
    expected_inbound: InboundExpectation,
    expected_outbound: BTreeMap<WriteOperation, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InboundExpectation {
    untracked: Value,
    tracked: Value,
}

#[derive(Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
enum WriteOperation {
    Create,
    Update,
    Upsert,
}

#[test]
fn catalog_fixtures() -> Result<()> {
    let suite_path = std::env::var_os("TAKU_TRANSPORT_SUITE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transport/suite.yaml")
        });
    let suite: Suite = serde_yaml::from_slice(
        &fs::read(&suite_path).with_context(|| format!("read suite {}", suite_path.display()))?,
    )
    .with_context(|| format!("parse suite {}", suite_path.display()))?;
    ensure!(!suite.fixtures.is_empty(), "suite has no fixtures");
    for fixture in suite.fixtures {
        let catalog_path = suite_path
            .parent()
            .context("suite has no parent directory")?
            .join(&fixture.catalog);
        let catalog: ResourceTypeCatalog = serde_yaml::from_slice(
            &fs::read(&catalog_path)
                .with_context(|| format!("read catalog {}", catalog_path.display()))?,
        )
        .with_context(|| format!("parse catalog {}", catalog_path.display()))?;
        ensure!(
            !fixture.cases.is_empty(),
            "{} has no cases",
            catalog_path.display()
        );
        for case in fixture.cases {
            let label = format!(
                "{}: {}: {} variant {} ({})",
                suite_path.display(),
                catalog_path.display(),
                case.resource_type,
                case.variant,
                case.name,
            );
            let resource_type = catalog
                .resource_types
                .get(&case.resource_type)
                .and_then(|variants| variants.get(case.variant))
                .with_context(|| format!("{label}: resource type/variant not found"))?;
            ensure!(
                !case.expected_outbound.is_empty(),
                "{label}: no write operations"
            );
            for (track, expected) in [
                (false, &case.expected_inbound.untracked),
                (true, &case.expected_inbound.tracked),
            ] {
                let label = format!("{label}: metadata_track={track}");
                let canonical = inbound(&case.input, resource_type, track)
                    .with_context(|| format!("{label}: inbound"))?;
                assert_eq!(&canonical, expected, "{label}: inbound");
                for (write, expected) in &case.expected_outbound {
                    let operation = match write {
                        WriteOperation::Create => &resource_type.operations.create,
                        WriteOperation::Update => &resource_type.operations.update,
                        WriteOperation::Upsert => &resource_type.operations.upsert,
                    }
                    .as_ref()
                    .with_context(|| format!("{label}: missing {write:?} operation"))?;
                    let actual = outbound(&canonical, resource_type, Some(operation))
                        .with_context(|| format!("{label}: outbound {write:?}"))?;
                    assert_eq!(&actual, expected, "{label}: outbound {write:?}");
                    eprintln!("PASS {label}: inbound + outbound {write:?}");
                }
            }
        }
    }
    Ok(())
}
