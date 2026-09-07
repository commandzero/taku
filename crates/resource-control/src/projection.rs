use crate::{FilesystemFormat, FilesystemProjection, FrontmatterMarkdown, ReferencedFiles};
use anyhow::{Context, Result, bail};
use pulldown_cmark::{Event, MetadataBlockKind, Options, Parser, Tag, TagEnd};
use serde_json::{Map, Value};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub(crate) fn merge(directory: &Path, projection: &FilesystemProjection) -> Result<Value> {
    match projection.merge {
        FilesystemFormat::FrontmatterMarkdown => merge_frontmatter_markdown(
            directory,
            projection
                .frontmatter_markdown
                .as_ref()
                .context("frontmatter_markdown projection has no configuration")?,
        ),
    }
}

pub(crate) fn split(
    directory: &Path,
    projection: &FilesystemProjection,
    resource: &Value,
) -> Result<()> {
    match projection.split {
        FilesystemFormat::FrontmatterMarkdown => split_frontmatter_markdown(
            directory,
            projection
                .frontmatter_markdown
                .as_ref()
                .context("frontmatter_markdown projection has no configuration")?,
            resource,
        ),
    }
}

fn split_frontmatter_markdown(
    directory: &Path,
    config: &FrontmatterMarkdown,
    resource: &Value,
) -> Result<()> {
    let mut frontmatter = resource.clone();
    // Taku-managed canonical state identifies the Resource but is not
    // user-authored frontmatter in a filesystem projection.
    remove_pointer(&mut frontmatter, crate::canonical::TAKU_NAMESPACE_POINTER)?;
    let body = remove_pointer(&mut frontmatter, &config.body_pointer)?
        .unwrap_or_else(|| Value::String(String::new()))
        .as_str()
        .context("frontmatter_markdown body must be a string")?
        .to_owned();
    let referenced = remove_pointer(&mut frontmatter, &config.referenced_files.pointer)?
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let referenced = referenced
        .as_array()
        .context("frontmatter_markdown referenced files must be an array")?;
    if !frontmatter.is_object() {
        bail!("frontmatter_markdown passthrough value must be an object");
    }

    let parent = directory
        .parent()
        .context("projected Resource destination has no parent")?;
    fs::create_dir_all(parent)?;
    let file_name = directory
        .file_name()
        .and_then(|value| value.to_str())
        .context("projected Resource destination is not UTF-8")?;
    let staging = parent.join(format!(".{file_name}.{}.taku.tmp", std::process::id()));
    if staging.exists() {
        bail!("projected Resource staging path already exists");
    }
    fs::create_dir(&staging)?;
    let written = (|| -> Result<()> {
        let yaml = serde_yaml::to_string(&frontmatter)?;
        fs::write(
            staging.join(&config.document),
            format!("---\n{yaml}---\n{body}"),
        )?;
        write_referenced_files(
            &staging,
            referenced,
            &config.referenced_files,
            &config.document,
        )
    })();
    if let Err(error) = written {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    replace_directory(&staging, directory)
}

fn write_referenced_files(
    root: &Path,
    values: &[Value],
    config: &ReferencedFiles,
    document: &str,
) -> Result<()> {
    let mut destinations = std::collections::BTreeSet::new();
    for value in values {
        let relative_path = value
            .pointer(&config.path_pointer)
            .and_then(Value::as_str)
            .context("referenced file path is missing or is not a string")?;
        let name = value
            .pointer(&config.name_pointer)
            .and_then(Value::as_str)
            .context("referenced file name is missing or is not a string")?;
        let content = value
            .pointer(&config.content_pointer)
            .and_then(Value::as_str)
            .context("referenced file content is missing or is not a string")?;
        let relative_directory = safe_relative_directory(relative_path)?;
        if name.is_empty()
            || matches!(name, "." | "..")
            || name.contains('/')
            || name.contains('\\')
        {
            bail!("referenced file name is not one safe path segment");
        }
        let relative = relative_directory.join(format!("{name}.{}", config.extension));
        if relative == Path::new(document) {
            bail!("referenced file conflicts with the primary document");
        }
        let collision_key = relative.to_string_lossy().to_ascii_lowercase();
        if !destinations.insert(collision_key) {
            bail!("referenced files have a case-insensitive path collision");
        }
        let destination = root.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(destination, content)?;
    }
    Ok(())
}

fn safe_relative_directory(value: &str) -> Result<PathBuf> {
    let mut safe = PathBuf::new();
    for component in Path::new(value).components() {
        match component {
            Component::CurDir => {}
            Component::Normal(segment) => safe.push(segment),
            _ => bail!("referenced file path must remain inside its Resource directory"),
        }
    }
    Ok(safe)
}

fn replace_directory(staging: &Path, destination: &Path) -> Result<()> {
    if !destination.exists() {
        fs::rename(staging, destination)?;
        return Ok(());
    }
    reject_symlink(destination)?;
    if !fs::symlink_metadata(destination)?.is_dir() {
        bail!("projected Resource destination is not a directory");
    }
    let backup = destination.with_extension(format!("{}.taku.previous", std::process::id()));
    if backup.exists() {
        bail!("projected Resource backup path already exists");
    }
    fs::rename(destination, &backup)?;
    if let Err(error) = fs::rename(staging, destination) {
        fs::rename(&backup, destination)
            .context("projection replacement failed and rollback could not restore the Resource")?;
        return Err(error.into());
    }
    fs::remove_dir_all(backup)?;
    Ok(())
}

fn merge_frontmatter_markdown(directory: &Path, config: &FrontmatterMarkdown) -> Result<Value> {
    reject_symlink(directory)?;
    let document_path = directory.join(&config.document);
    reject_symlink(&document_path)?;
    let markdown = fs::read_to_string(&document_path).with_context(|| {
        format!(
            "failed to read projected Resource {}",
            document_path.display()
        )
    })?;
    let (frontmatter, body) = parse_frontmatter(&markdown)?;
    let mut resource: Value = serde_yaml::from_str(frontmatter)
        .context("invalid YAML frontmatter in projected Resource")?;
    if !resource.is_object() {
        bail!("projected Resource frontmatter must be a YAML mapping");
    }
    set_pointer(
        &mut resource,
        &config.body_pointer,
        Value::String(body.to_owned()),
    )?;

    let mut paths = Vec::new();
    collect_files(directory, directory, &document_path, &mut paths)?;
    paths.sort();
    let referenced = paths
        .into_iter()
        .map(|path| referenced_value(directory, &path, &config.referenced_files))
        .collect::<Result<Vec<_>>>()?;
    set_pointer(
        &mut resource,
        &config.referenced_files.pointer,
        Value::Array(referenced),
    )?;
    Ok(resource)
}

fn parse_frontmatter(markdown: &str) -> Result<(&str, &str)> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    let mut metadata_range = None;
    let mut body_start = None;
    for (event, range) in Parser::new_ext(markdown, options).into_offset_iter() {
        match event {
            Event::Start(Tag::MetadataBlock(MetadataBlockKind::YamlStyle)) => {
                metadata_range = Some(range);
            }
            Event::End(TagEnd::MetadataBlock(MetadataBlockKind::YamlStyle)) => {
                body_start = Some(range.end);
                break;
            }
            _ => {}
        }
    }
    let metadata_range =
        metadata_range.context("projected Resource is missing YAML frontmatter")?;
    let body_start = body_start.context("projected Resource has unterminated YAML frontmatter")?;
    let block = &markdown[metadata_range];
    let frontmatter = block
        .strip_prefix("---\r\n")
        .or_else(|| block.strip_prefix("---\n"))
        .context("projected Resource has invalid YAML frontmatter delimiters")?;
    let frontmatter = frontmatter
        .strip_suffix("\r\n---")
        .or_else(|| frontmatter.strip_suffix("\n---"))
        .context("projected Resource has invalid YAML frontmatter delimiters")?;
    let body = markdown[body_start..]
        .strip_prefix("\r\n")
        .or_else(|| markdown[body_start..].strip_prefix('\n'))
        .unwrap_or(&markdown[body_start..]);
    Ok((frontmatter, body))
}

fn collect_files(
    canonical_root: &Path,
    directory: &Path,
    document: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        reject_symlink(&path)?;
        let canonical = path.canonicalize()?;
        if !canonical.starts_with(canonical_root.canonicalize()?) {
            bail!(
                "projected Resource path escapes its directory: {}",
                path.display()
            );
        }
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            collect_files(canonical_root, &path, document, files)?;
        } else if metadata.is_file() && path != document {
            files.push(path);
        } else if !metadata.is_file() {
            bail!("projected Resource contains a special filesystem entry");
        }
    }
    Ok(())
}

fn referenced_value(root: &Path, path: &Path, config: &ReferencedFiles) -> Result<Value> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .context("projected referenced file has no UTF-8 extension")?;
    if extension != config.extension {
        bail!(
            "projected referenced file {} does not use configured .{} extension",
            path.display(),
            config.extension
        );
    }
    let relative = path.strip_prefix(root)?;
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    let relative_path = if parent.as_os_str().is_empty() {
        String::new()
    } else {
        format!("./{}", path_string(parent)?)
    };
    let name = relative
        .file_stem()
        .and_then(|value| value.to_str())
        .context("projected referenced filename is not UTF-8")?;
    let mut value = Value::Object(Map::new());
    set_pointer(
        &mut value,
        &config.path_pointer,
        Value::String(relative_path),
    )?;
    set_pointer(
        &mut value,
        &config.name_pointer,
        Value::String(name.to_owned()),
    )?;
    set_pointer(
        &mut value,
        &config.content_pointer,
        Value::String(fs::read_to_string(path)?),
    )?;
    Ok(value)
}

fn path_string(path: &Path) -> Result<String> {
    path.components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_owned)
                .context("projected Resource path is not UTF-8"),
            _ => bail!("projected Resource contains an unsafe path"),
        })
        .collect::<Result<Vec<_>>>()
        .map(|parts| parts.join("/"))
}

fn reject_symlink(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).with_context(|| {
        format!(
            "failed to inspect projected Resource path {}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() {
        bail!(
            "projected Resource cannot contain symlinks: {}",
            path.display()
        );
    }
    Ok(())
}

pub(crate) fn set_pointer(value: &mut Value, pointer: &str, inserted: Value) -> Result<()> {
    let segments = pointer_segments(pointer)?;
    let (last, parents) = segments
        .split_last()
        .context("JSON pointer cannot address the document root")?;
    let mut current = value;
    for segment in parents {
        let object = current
            .as_object_mut()
            .context("JSON pointer parent is not an object")?;
        current = object
            .entry(segment.clone())
            .or_insert_with(|| Value::Object(Map::new()));
    }
    current
        .as_object_mut()
        .context("JSON pointer parent is not an object")?
        .insert(last.clone(), inserted);
    Ok(())
}

pub(crate) fn remove_pointer(value: &mut Value, pointer: &str) -> Result<Option<Value>> {
    let segments = pointer_segments(pointer)?;
    let (last, parents) = segments
        .split_last()
        .context("JSON pointer cannot address the document root")?;
    let mut current = value;
    for segment in parents {
        let Some(next) = current.as_object_mut().and_then(|map| map.get_mut(segment)) else {
            return Ok(None);
        };
        current = next;
    }
    Ok(current.as_object_mut().and_then(|map| map.remove(last)))
}

fn pointer_segments(pointer: &str) -> Result<Vec<String>> {
    if !pointer.starts_with('/') {
        bail!("invalid JSON pointer");
    }
    Ok(pointer[1..]
        .split('/')
        .map(|segment| segment.replace("~1", "/").replace("~0", "~"))
        .collect())
}
