use anyhow::{Result, bail};
use serde_json::{Map, Value};

pub fn insert_pointer(root: &mut Value, pointer: &str, inserted: Value) -> Result<()> {
    let tokens: Vec<String> = pointer
        .split('/')
        .skip(1)
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect();
    if tokens.is_empty() {
        *root = inserted;
        return Ok(());
    }
    let mut current = root;
    for token in &tokens[..tokens.len() - 1] {
        if !current.is_object() {
            *current = Value::Object(Map::new());
        }
        current = current
            .as_object_mut()
            .unwrap()
            .entry(token)
            .or_insert_with(|| Value::Object(Map::new()));
    }
    if !current.is_object() {
        bail!("Transformation pointer parent is not an object");
    }
    current
        .as_object_mut()
        .unwrap()
        .insert(tokens.last().unwrap().clone(), inserted);
    Ok(())
}

pub fn remove_pointer(root: &mut Value, pointer: &str) -> Result<()> {
    let mut tokens: Vec<&str> = pointer.split('/').skip(1).collect();
    let Some(last) = tokens.pop() else {
        bail!("cannot remove document root");
    };
    let mut current = root;
    for raw in tokens {
        let token = raw.replace("~1", "/").replace("~0", "~");
        let next = match current {
            Value::Object(map) => map.get_mut(&token),
            Value::Array(values) => token
                .parse::<usize>()
                .ok()
                .and_then(|index| values.get_mut(index)),
            _ => None,
        };
        let Some(next) = next else {
            return Ok(());
        };
        current = next;
    }
    let last = last.replace("~1", "/").replace("~0", "~");
    match current {
        Value::Object(map) => {
            map.remove(&last);
        }
        Value::Array(values) => {
            if let Ok(index) = last.parse::<usize>()
                && index < values.len()
            {
                values.remove(index);
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn remove_pointers(root: &mut Value, pointers: &[String]) -> Result<()> {
    let mut pointers = pointers.iter().map(String::as_str).collect::<Vec<_>>();
    pointers.sort_by(|left, right| {
        let left = left.split('/').skip(1);
        let right = right.split('/').skip(1);
        for (left, right) in left.zip(right) {
            if left == right {
                continue;
            }
            return match (left.parse::<usize>(), right.parse::<usize>()) {
                (Ok(left), Ok(right)) => right.cmp(&left),
                _ => left.cmp(right),
            };
        }
        std::cmp::Ordering::Equal
    });
    for pointer in pointers {
        remove_pointer(root, pointer)?;
    }
    Ok(())
}
