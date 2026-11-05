use super::{
    CONTEXT_FIELD_HARD_LIMIT, DISPATCH_FUNCTION_HARD_LIMIT, FACADE_HARD_LIMIT, REGULAR_HARD_LIMIT,
};
use crate::Result;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

pub(super) type ArchitectureAllowlistKey = (String, String, String);
pub(super) type ArchitectureAllowlist = BTreeMap<ArchitectureAllowlistKey, ArchitectureException>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ArchitectureException {
    pub(super) reason: String,
    pub(super) review_trigger: String,
    pub(super) baseline: Option<usize>,
}

fn metric_hard_limit(rule: &str) -> Option<usize> {
    match rule {
        "regular_module_lines" => Some(REGULAR_HARD_LIMIT),
        "facade_lines" => Some(FACADE_HARD_LIMIT),
        "dispatch_function_lines" => Some(DISPATCH_FUNCTION_HARD_LIMIT),
        "context_fields" => Some(CONTEXT_FIELD_HARD_LIMIT),
        "production_glob_import" => Some(0),
        _ => None,
    }
}

pub(super) fn parse_architecture_allowlist(
    root: &Path,
    payload: Value,
) -> Result<ArchitectureAllowlist> {
    let entries = payload
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("architecture allowlist entries must be an array")?;
    let mut result = BTreeMap::new();
    for entry in entries {
        let object = entry
            .as_object()
            .ok_or("architecture allowlist entry must be an object")?;
        if object.keys().any(|key| {
            ![
                "path",
                "rule",
                "reason",
                "review_trigger",
                "baseline",
                "subject",
            ]
            .contains(&key.as_str())
        }) {
            return Err("architecture allowlist entry has an unknown field".into());
        }
        let path = object
            .get("path")
            .and_then(Value::as_str)
            .ok_or("architecture allowlist path missing")?;
        let rule = object
            .get("rule")
            .and_then(Value::as_str)
            .ok_or("architecture allowlist rule missing")?;
        let reason = object
            .get("reason")
            .and_then(Value::as_str)
            .ok_or("architecture allowlist reason missing")?;
        let review_trigger = object
            .get("review_trigger")
            .and_then(Value::as_str)
            .ok_or("architecture allowlist review trigger missing")?;
        if reason.trim().is_empty() || review_trigger.trim().is_empty() {
            return Err(
                "architecture allowlist reason and review trigger must be non-empty".into(),
            );
        }
        let subject = match object.get("subject") {
            Some(Value::String(value)) if !value.trim().is_empty() => value.trim(),
            Some(_) => {
                return Err("architecture allowlist subject must be a non-empty string".into());
            }
            None => "",
        };
        let baseline = match object.get("baseline") {
            Some(Value::Number(value)) => value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("architecture allowlist baseline must be a non-negative integer")?,
            Some(_) => {
                return Err(
                    "architecture allowlist baseline must be a non-negative integer".into(),
                );
            }
            None => 0,
        };
        let baseline = object.contains_key("baseline").then_some(baseline);
        let hard_limit = metric_hard_limit(rule);
        match (rule, hard_limit, baseline, subject.is_empty()) {
            ("unsafe_boundary", None, None, true) => {}
            (_, Some(limit), Some(baseline), _) if baseline > limit => {
                if matches!(rule, "dispatch_function_lines" | "context_fields")
                    && subject.is_empty()
                {
                    return Err(
                        format!("architecture allowlist {rule} entry requires a subject").into(),
                    );
                }
                if matches!(
                    rule,
                    "regular_module_lines" | "facade_lines" | "production_glob_import"
                ) && !subject.is_empty()
                {
                    return Err(format!(
                        "architecture allowlist {rule} entry must not have a subject"
                    )
                    .into());
                }
            }
            ("unsafe_boundary", None, _, _) => {
                return Err(
                    "unsafe_boundary entries must not have a metric baseline or subject".into(),
                );
            }
            (_, Some(_), None, _) => {
                return Err(
                    format!("architecture allowlist {rule} entry requires a baseline").into(),
                );
            }
            (_, Some(limit), Some(baseline), _) => {
                return Err(format!("architecture allowlist {rule} baseline {baseline} must exceed hard limit {limit}").into());
            }
            _ => return Err(format!("unknown architecture allowlist rule: {rule}").into()),
        }
        if path.trim() != path
            || path.contains('\\')
            || Path::new(path).is_absolute()
            || Path::new(path).components().any(|component| {
                matches!(
                    component,
                    std::path::Component::CurDir
                        | std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(format!(
                "architecture allowlist path must be repository-relative and normalized: {path}"
            )
            .into());
        }
        if !root.join(path).is_file() {
            return Err(format!("architecture allowlist path does not exist: {path}").into());
        }
        let key = (path.to_owned(), rule.to_owned(), subject.to_owned());
        if result
            .insert(
                key,
                ArchitectureException {
                    reason: reason.trim().to_owned(),
                    review_trigger: review_trigger.trim().to_owned(),
                    baseline,
                },
            )
            .is_some()
        {
            return Err(format!("duplicate architecture allowlist entry: {path} ({rule})").into());
        }
    }
    Ok(result)
}

pub(super) fn load_architecture_allowlist(root: &Path) -> Result<ArchitectureAllowlist> {
    let path = root.join(".config/rust_architecture_allowlist.json");
    if !path.is_file() {
        return Err(format!("architecture allowlist is missing: {}", path.display()).into());
    }
    let payload: Value = serde_json::from_slice(&fs::read(path)?)?;
    parse_architecture_allowlist(root, payload)
}

pub(super) fn architecture_record(
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
    allowlist: &ArchitectureAllowlist,
    path: &str,
    rule: &str,
    message: impl Into<String>,
) {
    let message = message.into();
    if let Some(debt) = allowlist.get(&(path.to_owned(), rule.to_owned(), String::new())) {
        warnings.push(format!(
            "[{rule}] {path}: {message}; temporary allowlist: {} (review: {})",
            debt.reason, debt.review_trigger
        ));
    } else {
        errors.push(format!("[{rule}] {path}: {message}"));
    }
}

#[derive(Clone, Copy)]
pub(super) struct ArchitectureMetric<'a> {
    pub(super) path: &'a str,
    pub(super) rule: &'a str,
    pub(super) subject: &'a str,
    pub(super) observed: usize,
    pub(super) unit: &'a str,
    pub(super) hard_limit: usize,
}

pub(super) fn architecture_metric_record(
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
    allowlist: &ArchitectureAllowlist,
    used: &mut BTreeSet<ArchitectureAllowlistKey>,
    metric: ArchitectureMetric<'_>,
) {
    let ArchitectureMetric {
        path,
        rule,
        subject,
        observed,
        unit,
        hard_limit,
    } = metric;
    let key = (path.to_owned(), rule.to_owned(), subject.to_owned());
    let Some(exception) = allowlist.get(&key) else {
        let label = if subject.is_empty() {
            String::new()
        } else {
            format!(" {subject}")
        };
        errors.push(format!(
            "[{rule}] {path}:{label} observed {observed} {unit} (> hard limit {hard_limit}) without an exact baseline exception"
        ));
        return;
    };
    used.insert(key);
    let baseline = exception
        .baseline
        .expect("metric allowlist schema requires a baseline");
    if observed > baseline {
        errors.push(format!(
            "[architecture_ratchet] [{rule}] {path}:{subject} observed {observed} {unit}, above baseline {baseline}"
        ));
        return;
    }
    let label = if subject.is_empty() {
        String::new()
    } else {
        format!("{subject}: ")
    };
    warnings.push(format!(
        "[{rule}] {path}: {label}{observed} {unit} (> hard limit {hard_limit}; baseline {baseline}); temporary allowlist: {} (review: {})",
        exception.reason, exception.review_trigger
    ));
}

pub(super) fn stale_metric_allowlist_entries(
    allowlist: &ArchitectureAllowlist,
    used: &BTreeSet<ArchitectureAllowlistKey>,
) -> Vec<String> {
    allowlist
        .iter()
        .filter(|((_, rule, _), _)| rule != "unsafe_boundary")
        .filter(|(key, _)| !used.contains(*key))
        .map(|((path, rule, subject), _)| {
            format!(
                "[stale_architecture_allowlist_entry] {path} ({rule}{}) has no live violation above its hard limit",
                if subject.is_empty() {
                    String::new()
                } else {
                    format!(", subject {subject}")
                }
            )
        })
        .collect()
}
