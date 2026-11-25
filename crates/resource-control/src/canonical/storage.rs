use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::fs;
use std::path::Path;

pub fn parse_resource(path: &Path) -> Result<Value> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read Resource {}", path.display()))?;
    match path.extension().and_then(|e| e.to_str()) {
        Some("yaml" | "yml") => serde_yaml::from_str(&text).context("invalid YAML Resource"),
        _ => json5::from_str(&normalize_triple_quotes(&text)).context("invalid JSON Resource"),
    }
}

fn write_resource_with_embedded_yaml(
    path: &Path,
    value: &Value,
    embedded_yaml: &[String],
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = if matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("yaml" | "yml")
    ) {
        serde_yaml::to_string(value)?
    } else {
        render_json5(value, "", 0, embedded_yaml) + "\n"
    };
    let temporary = path.with_extension("taku.tmp");
    fs::write(&temporary, text)?;
    fs::rename(temporary, path)?;
    Ok(())
}

pub(crate) fn write_canonical_resource(
    path: &Path,
    value: &Value,
    resource_type: &crate::ResourceType,
) -> Result<()> {
    if let Some(projection) = &resource_type.filesystem {
        crate::projection::split(path, projection, value)
    } else {
        let embedded_yaml = resource_type
            .transformations
            .iter()
            .filter_map(|transformation| match transformation {
                crate::Transformation::EmbeddedYaml { pointer } => Some(pointer.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        write_resource_with_embedded_yaml(path, value, &embedded_yaml)
    }
}

/// Render repository JSON as JSON5 when configured fields contain embedded YAML.
/// The YAML remains a string, but multi-line content is emitted with the JSON5
/// triple-quote extension so it can be reviewed and edited without escaped lines.
fn render_json5(value: &Value, pointer: &str, indent: usize, embedded_yaml: &[String]) -> String {
    match value {
        Value::String(text)
            if use_triple_quotes(text)
                && embedded_yaml.iter().any(|candidate| candidate == pointer) =>
        {
            format!("\"\"\"{text}\"\"\"")
        }
        Value::Array(values) if values.is_empty() => "[]".to_owned(),
        Value::Array(values) => {
            let child_indent = indent + 2;
            let padding = " ".repeat(child_indent);
            let closing_padding = " ".repeat(indent);
            let entries = values
                .iter()
                .enumerate()
                .map(|(index, item)| {
                    let child_pointer = format!("{pointer}/{index}");
                    format!(
                        "{padding}{}",
                        render_json5(item, &child_pointer, child_indent, embedded_yaml)
                    )
                })
                .collect::<Vec<_>>();
            format!("[\n{}\n{closing_padding}]", entries.join(",\n"))
        }
        Value::Object(values) if values.is_empty() => "{}".to_owned(),
        Value::Object(values) => {
            let child_indent = indent + 2;
            let padding = " ".repeat(child_indent);
            let closing_padding = " ".repeat(indent);
            let entries = values
                .iter()
                .map(|(key, item)| {
                    let escaped_key = key.replace('~', "~0").replace('/', "~1");
                    let child_pointer = format!("{pointer}/{escaped_key}");
                    format!(
                        "{padding}{}: {}",
                        serde_json::to_string(key).unwrap(),
                        render_json5(item, &child_pointer, child_indent, embedded_yaml)
                    )
                })
                .collect::<Vec<_>>();
            format!("{{\n{}\n{closing_padding}}}", entries.join(",\n"))
        }
        _ => serde_json::to_string(value).unwrap(),
    }
}

fn use_triple_quotes(text: &str) -> bool {
    text.contains('\n') && !text.contains("\"\"\"")
}

pub(crate) fn remove_canonical_resource(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        bail!("symlinked Resource cannot be removed: {}", path.display());
    }
    if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else if metadata.is_file() {
        fs::remove_file(path)?;
    } else {
        bail!(
            "Resource is not a regular file or directory: {}",
            path.display()
        );
    }
    Ok(())
}

fn normalize_triple_quotes(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("\"\"\"") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 3..];
        if let Some(end) = after.find("\"\"\"") {
            output.push_str(&serde_json::to_string(&after[..end]).unwrap());
            rest = &after[end + 3..];
        } else {
            output.push_str(&rest[start..]);
            return output;
        }
    }
    output.push_str(rest);
    output
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_triple_quotes, parse_resource, render_json5, write_resource_with_embedded_yaml,
    };
    use serde_json::{Value, json};

    #[test]
    fn embedded_yaml_is_readable_json5_and_round_trips_without_losing_comments() {
        let value = json!({
            "id": "workflow-1",
            "yaml": "# retained comment\nname: User Diagnostic ID Fetcher\nsteps:\n  - name: fetch\n"
        });

        let text = render_json5(&value, "", 0, &["/yaml".to_owned()]);

        assert!(
            text.contains("\"yaml\": \"\"\"# retained comment\nname: User Diagnostic ID Fetcher")
        );
        assert!(!text.contains("\\nname: User Diagnostic ID Fetcher"));
        let parsed: Value = json5::from_str(&normalize_triple_quotes(&text)).unwrap();
        assert_eq!(parsed, value);
    }

    #[test]
    fn embedded_yaml_containing_triple_quotes_uses_standard_json_escaping() {
        let value = json!({"yaml": "note: '\"\"\"'\n"});

        let text = render_json5(&value, "", 0, &["/yaml".to_owned()]);

        assert!(!text.contains("\"yaml\": \"\"\""));
        let parsed: Value = json5::from_str(&normalize_triple_quotes(&text)).unwrap();
        assert_eq!(parsed, value);
    }

    #[test]
    fn canonical_writer_persists_embedded_yaml_as_a_triple_quoted_json5_value() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workflow.json");
        let value = json!({"yaml": "name: Workflow\nsteps: []\n"});

        write_resource_with_embedded_yaml(&path, &value, &["/yaml".to_owned()]).unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"yaml\": \"\"\"name: Workflow\nsteps: []\n\"\"\""));
        assert_eq!(parse_resource(&path).unwrap(), value);
    }
}
