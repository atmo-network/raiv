//! Readable presentation of accepted Memory JSON: flattened entries and boundary changes.
//! Display only; never parsed back, edited, checkpointed or shown to the guest.

use serde_json::Value;

const MAX_LINES: usize = 12;

const MAX_VALUE: usize = 72;

/// Leaf entries as `plane.key.sub` paths; arrays and scalars are leaves in compact JSON.
fn entries(json: &str) -> Vec<(String, String)> {
    let Ok(root) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut stack = vec![(String::new(), &root)];
    while let Some((path, value)) = stack.pop() {
        match value {
            Value::Object(map) if !map.is_empty() || path.is_empty() => {
                for (key, child) in map.iter().rev() {
                    let child_path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    stack.push((child_path, child));
                }
            }
            _ => out.push((path, shorten(&value.to_string()))),
        }
    }
    out
}

fn shorten(value: &str) -> String {
    match value.char_indices().nth(MAX_VALUE) {
        Some((at, _)) => format!("{}…", &value[..at]),
        None => value.to_owned(),
    }
}

fn capped(mut lines: Vec<String>) -> String {
    if lines.len() > MAX_LINES {
        let more = lines.len() - MAX_LINES + 1;
        lines.truncate(MAX_LINES - 1);
        lines.push(format!(
            "… {more} more; ./run.sh --status prints the full Memory"
        ));
    }
    lines.join("\n")
}

/// Every entry of accepted Memory, for the restored-session view.
pub(super) fn summary(json: &str) -> String {
    capped(
        entries(json)
            .into_iter()
            .map(|(path, value)| format!("{path} = {value}"))
            .collect(),
    )
}

/// Added (+), changed (~) and removed (-) entries between two accepted Memory values.
pub(super) fn changes(before: &str, after: &str) -> String {
    let old = entries(before);
    let new = entries(after);
    let mut lines = Vec::new();
    for (path, value) in &new {
        match old.iter().find(|(p, _)| p == path) {
            None => lines.push(format!("+ {path} = {value}")),
            Some((_, prior)) if prior != value => lines.push(format!("~ {path} = {value}")),
            Some(_) => {}
        }
    }
    for (path, _) in &old {
        if !new.iter().any(|(p, _)| p == path) {
            lines.push(format!("- {path}"));
        }
    }
    capped(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_flatten_objects_keep_arrays_whole_and_diff_marks_each_change() {
        let before = r#"{"intents":{"goal":"ship","old":1},"tactics":{"steps":["a"]}}"#;
        let after =
            r#"{"intents":{"goal":"ship 0.0.2","owner":{"name":"Lev"}},"tactics":{"steps":["a"]}}"#;
        assert_eq!(
            summary(before),
            "intents.goal = \"ship\"\nintents.old = 1\ntactics.steps = [\"a\"]"
        );
        assert_eq!(
            changes(before, after),
            "~ intents.goal = \"ship 0.0.2\"\n+ intents.owner.name = \"Lev\"\n- intents.old"
        );
        assert_eq!(summary("{}"), "");
        assert_eq!(
            changes("{}", r#"{"intents":{"x":true}}"#),
            "+ intents.x = true"
        );
    }

    #[test]
    fn long_values_and_many_entries_are_bounded() {
        let long = format!(r#"{{"intents":{{"note":"{}"}}}}"#, "é".repeat(200));
        let line = summary(&long);
        assert!(line.ends_with('…') && line.chars().count() < MAX_VALUE + 20);
        let many = format!(
            "{{\"tactics\":{{{}}}}}",
            (0..30)
                .map(|n| format!("\"k{n:02}\":{n}"))
                .collect::<Vec<_>>()
                .join(",")
        );
        let lines = summary(&many);
        assert_eq!(lines.lines().count(), MAX_LINES);
        assert!(lines.ends_with("./run.sh --status prints the full Memory"));
    }
}
