use crate::canonical::pointer_string;
use crate::transport::{INTERNAL_CURSOR_POINTER, remove_pointer};
use crate::{Operation, Pagination};
use anyhow::{Result, bail};
use serde_json::Value;
use std::collections::BTreeSet;

pub(super) fn read(
    operation: &Operation,
    fetch: impl Fn(&Operation) -> Result<Option<Vec<Value>>>,
) -> Result<Vec<Value>> {
    match &operation.pagination {
        Some(Pagination::PageSize {
            page_parameter,
            size_parameter,
            size,
            max_pages,
        }) => read_sized(
            operation,
            fetch,
            SizedPages {
                page_parameter,
                size_parameter,
                size: *size,
                max_pages: *max_pages,
            },
        ),
        Some(Pagination::Cursor {
            cursor_parameter,
            max_pages,
            ..
        }) => read_cursor(operation, fetch, cursor_parameter, *max_pages),
        None => Ok(fetch(operation)?.unwrap_or_default()),
    }
}

struct SizedPages<'a> {
    page_parameter: &'a str,
    size_parameter: &'a str,
    size: usize,
    max_pages: usize,
}

fn read_sized(
    operation: &Operation,
    fetch: impl Fn(&Operation) -> Result<Option<Vec<Value>>>,
    pages: SizedPages<'_>,
) -> Result<Vec<Value>> {
    let SizedPages {
        page_parameter,
        size_parameter,
        size,
        max_pages,
    } = pages;
    let mut resources = Vec::new();
    for page in 1..=max_pages {
        let mut operation = operation.clone();
        append_query_parameter(&mut operation.path, page_parameter, &page.to_string());
        append_query_parameter(&mut operation.path, size_parameter, &size.to_string());
        let Some(values) = fetch(&operation)? else {
            break;
        };
        let count = values.len();
        resources.extend(values);
        if count < size {
            break;
        }
        if page == max_pages {
            bail!("pagination exceeded configured max_pages");
        }
    }
    Ok(resources)
}

fn read_cursor(
    operation: &Operation,
    fetch: impl Fn(&Operation) -> Result<Option<Vec<Value>>>,
    cursor_parameter: &str,
    max_pages: usize,
) -> Result<Vec<Value>> {
    let mut resources = Vec::new();
    let mut cursor = None::<String>;
    let mut seen_cursors = BTreeSet::new();
    for page in 0..max_pages {
        let mut operation = operation.clone();
        if let Some(value) = &cursor {
            append_query_parameter(
                &mut operation.path,
                cursor_parameter,
                &urlencoding::encode(value),
            );
        }
        let Some(mut values) = fetch(&operation)? else {
            break;
        };
        let next = take_cursor(&mut values)?;
        resources.extend(values);
        let Some(next) = next else { break };
        if !seen_cursors.insert(next.clone()) {
            bail!("cursor pagination repeated a continuation cursor");
        }
        cursor = Some(next);
        if page + 1 == max_pages {
            bail!("pagination exceeded configured max_pages");
        }
    }
    Ok(resources)
}

fn take_cursor(values: &mut [Value]) -> Result<Option<String>> {
    let next = values
        .last()
        .and_then(|value| pointer_string(value, INTERNAL_CURSOR_POINTER));
    for value in values {
        remove_pointer(value, INTERNAL_CURSOR_POINTER)?;
    }
    Ok(next)
}

fn append_query_parameter(path: &mut String, name: &str, value: &str) {
    let separator = if path.contains('?') { '&' } else { '?' };
    path.push(separator);
    path.push_str(name);
    path.push('=');
    path.push_str(value);
}
