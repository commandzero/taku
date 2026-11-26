use super::{reserved_canonical_pointer, transformation_pointers, valid_json_pointer};
use crate::FrontmatterMarkdown;
use anyhow::{Context, Result, bail};
use std::path::Path;

pub(super) fn validate_metadata(resource_type: &crate::ResourceType, name: &str) -> Result<()> {
    use crate::model::json_pointers_overlap as pointers_overlap;
    let Some(metadata) = &resource_type.metadata else {
        return Ok(());
    };
    if metadata.fields.is_empty() {
        bail!("Resource Type {name} metadata fields cannot be empty");
    }
    for (index, pointer) in metadata.fields.iter().enumerate() {
        if !valid_json_pointer(pointer) {
            bail!(
                "Resource Type {name} metadata field {pointer:?} is not a canonical JSON pointer"
            );
        }
        if reserved_canonical_pointer(pointer) {
            bail!("Resource Type {name} metadata field {pointer} uses Taku's reserved namespace");
        }
        if metadata.fields[..index]
            .iter()
            .any(|other| pointers_overlap(pointer, other))
        {
            bail!("Resource Type {name} metadata fields overlap at {pointer}");
        }
        check_metadata_overlaps(resource_type, name, pointer)?;
    }
    Ok(())
}

fn check_metadata_overlaps(
    resource_type: &crate::ResourceType,
    name: &str,
    pointer: &str,
) -> Result<()> {
    use crate::model::json_pointers_overlap as overlaps;
    let required = std::iter::once(resource_type.id.pointer.as_str())
        .chain(resource_type.display_name.pointers())
        .chain(resource_type.sensitive_fields.iter().map(String::as_str));
    if required.into_iter().any(|other| overlaps(pointer, other)) {
        bail!(
            "Resource Type {name} metadata field {pointer} overlaps required or sensitive canonical state"
        );
    }
    if resource_type
        .transformations
        .iter()
        .flat_map(transformation_pointers)
        .any(|other| overlaps(pointer, other))
    {
        bail!("Resource Type {name} metadata field {pointer} overlaps a Transformation");
    }
    let frontmatter = resource_type
        .filesystem
        .as_ref()
        .and_then(|filesystem| filesystem.frontmatter_markdown.as_ref());
    if frontmatter.is_some_and(|frontmatter| {
        projection_pointers(frontmatter)
            .into_iter()
            .any(|(_, other)| overlaps(pointer, other))
    }) {
        bail!("Resource Type {name} metadata field {pointer} overlaps filesystem projection state");
    }
    Ok(())
}

pub(super) fn validate_filesystem(resource_type: &crate::ResourceType, owner: &str) -> Result<()> {
    let Some(filesystem) = &resource_type.filesystem else {
        return Ok(());
    };
    if filesystem.split != filesystem.merge {
        bail!("Resource Type {owner} split and merge formats must match");
    }
    let frontmatter = filesystem
        .frontmatter_markdown
        .as_ref()
        .context("frontmatter_markdown filesystem format requires its configuration")?;
    let document = Path::new(&frontmatter.document);
    if !relative_segment(document, &frontmatter.document) {
        bail!("Resource Type {owner} frontmatter document must be one relative path segment");
    }
    for (label, pointer) in projection_pointers(frontmatter) {
        if !pointer.starts_with('/') {
            bail!("Resource Type {owner} {label} must be a JSON pointer");
        }
        if reserved_canonical_pointer(pointer) {
            bail!("Resource Type {owner} {label} uses Taku's reserved namespace");
        }
    }
    let extension = &frontmatter.referenced_files.extension;
    if !valid_extension(extension) {
        bail!("Resource Type {owner} referenced file extension is invalid");
    }
    Ok(())
}

fn relative_segment(path: &Path, value: &str) -> bool {
    !path.is_absolute() && path.components().count() == 1 && !value.is_empty()
}

fn valid_extension(extension: &str) -> bool {
    !extension.is_empty() && !extension.starts_with('.') && !extension.contains(['/', '\\'])
}

fn projection_pointers(frontmatter: &FrontmatterMarkdown) -> [(&'static str, &str); 5] {
    [
        ("body_pointer", frontmatter.body_pointer.as_str()),
        (
            "referenced_files.pointer",
            frontmatter.referenced_files.pointer.as_str(),
        ),
        (
            "referenced_files.path_pointer",
            frontmatter.referenced_files.path_pointer.as_str(),
        ),
        (
            "referenced_files.name_pointer",
            frontmatter.referenced_files.name_pointer.as_str(),
        ),
        (
            "referenced_files.content_pointer",
            frontmatter.referenced_files.content_pointer.as_str(),
        ),
    ]
}
