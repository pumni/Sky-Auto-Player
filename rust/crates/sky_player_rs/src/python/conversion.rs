use super::{
    ActionKind, Arc, Bound, KeyActionInput, PHYSICAL_INSTRUMENT_SCAN_CODES, PyAny, PyBool, PyList,
    PyResult, PyTuple, PyTypeError, PyValueError, RuntimeSchedule,
};
use pyo3::types::PyAnyMethods;

pub(super) fn strict_sequence<'py>(
    value: &Bound<'py, PyAny>,
    field: &str,
) -> PyResult<Vec<Bound<'py, PyAny>>> {
    if !value.is_instance_of::<PyList>() && !value.is_instance_of::<PyTuple>() {
        return Err(PyTypeError::new_err(format!(
            "{field} must be a list or tuple"
        )));
    }
    value.try_iter()?.collect()
}

pub(super) fn strict_integer(value: &Bound<'_, PyAny>, field: &str) -> PyResult<i128> {
    if value.is_instance_of::<PyBool>() {
        return Err(PyTypeError::new_err(format!(
            "{field} must be an integer, not bool"
        )));
    }
    value
        .extract::<i128>()
        .map_err(|_| PyTypeError::new_err(format!("{field} must be an integer")))
}

pub(super) fn strict_u32(value: &Bound<'_, PyAny>, field: &str) -> PyResult<u32> {
    let integer = strict_integer(value, field)?;
    u32::try_from(integer)
        .map_err(|_| PyValueError::new_err(format!("{field} must be in 0..=u32::MAX")))
}

pub(super) fn strict_u64(value: &Bound<'_, PyAny>, field: &str) -> PyResult<u64> {
    let integer = strict_integer(value, field)?;
    u64::try_from(integer)
        .map_err(|_| PyValueError::new_err(format!("{field} must be in 0..=u64::MAX")))
}

pub(super) fn strict_scan_codes(
    value: &Bound<'_, PyAny>,
    field: &str,
    allowed: Option<&[u16]>,
) -> PyResult<smallvec::SmallVec<[u16; 4]>> {
    let items = strict_sequence(value, field)?;
    if items.is_empty() || items.len() > sky_dispatch_core::model::MAX_KEYS {
        return Err(PyValueError::new_err(format!(
            "{field} must contain between 1 and {} scan codes",
            sky_dispatch_core::model::MAX_KEYS
        )));
    }

    let mut result = smallvec::SmallVec::with_capacity(items.len());
    let mut seen = smallvec::SmallVec::<[u16; 15]>::new();
    for (index, item) in items.iter().enumerate() {
        let item_field = format!("{field}[{index}]");
        let integer = strict_integer(item, &item_field)?;
        let scan_code = u16::try_from(integer)
            .map_err(|_| PyValueError::new_err(format!("{item_field} must be in 0..=u16::MAX")))?;
        if seen.contains(&scan_code) {
            return Err(PyValueError::new_err(format!(
                "{field} contains duplicate scan code {scan_code}"
            )));
        }
        if let Some(allowed) = allowed
            && !allowed.contains(&scan_code)
        {
            return Err(PyValueError::new_err(format!(
                "{item_field} scan code {scan_code} is outside the prepared allowlist"
            )));
        }
        seen.push(scan_code);
        result.push(scan_code);
    }
    Ok(result)
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn parse_allowed_scan_codes(value: &Bound<'_, PyAny>) -> PyResult<Vec<u16>> {
    strict_scan_codes(
        value,
        "allowed_scan_codes",
        Some(&PHYSICAL_INSTRUMENT_SCAN_CODES),
    )
    .map(|v| v.into_vec())
}

pub(super) fn parse_actions(value: &Bound<'_, PyAny>) -> PyResult<Vec<KeyActionInput>> {
    parse_actions_with_allowlist(value, &PHYSICAL_INSTRUMENT_SCAN_CODES)
}

fn parse_actions_with_allowlist(
    value: &Bound<'_, PyAny>,
    allowed_scan_codes: &[u16],
) -> PyResult<Vec<KeyActionInput>> {
    let iter = value
        .try_iter()
        .map_err(|_| PyTypeError::new_err("actions must be an iterable"))?;

    let mut actions = Vec::new();
    let mut reason_interns = std::collections::HashMap::<String, Arc<str>>::new();

    for (position, item_res) in iter.enumerate() {
        if position >= sky_dispatch_core::compile::MAX_ACTIONS {
            return Err(PyValueError::new_err(format!(
                "actions exceeds the configured cap of {}",
                sky_dispatch_core::compile::MAX_ACTIONS
            )));
        }
        let item = item_res?;
        let tuple = item.cast::<PyTuple>().map_err(|_| {
            PyTypeError::new_err(format!(
                "actions[{position}] must be a 5-item tuple \
                 (source_action_index, kind, at_us, scan_codes, reason)"
            ))
        })?;
        if tuple.len()? != 5 {
            return Err(PyValueError::new_err(format!(
                "actions[{position}] must contain exactly 5 items"
            )));
        }

        let source_action_index = strict_u32(
            &tuple.get_item(0)?,
            &format!("actions[{position}].source_action_index"),
        )?;
        let kind_string = tuple
            .get_item(1)?
            .extract::<String>()
            .map_err(|_| PyTypeError::new_err(format!("actions[{position}].kind must be str")))?;
        let kind = match kind_string.as_str() {
            "down" => ActionKind::Down,
            "up" => ActionKind::Up,
            _ => {
                return Err(PyValueError::new_err(format!(
                    "actions[{position}].kind must be exactly 'down' or 'up'"
                )));
            }
        };
        let scheduled_us = strict_u64(&tuple.get_item(2)?, &format!("actions[{position}].at_us"))?;
        let scan_codes = strict_scan_codes(
            &tuple.get_item(3)?,
            &format!("actions[{position}].scan_codes"),
            Some(allowed_scan_codes),
        )?;
        let reason = tuple
            .get_item(4)?
            .extract::<String>()
            .map_err(|_| PyTypeError::new_err(format!("actions[{position}].reason must be str")))?;
        if reason.len() > sky_dispatch_core::compile::MAX_REASON_BYTES {
            return Err(PyValueError::new_err(format!(
                "actions[{position}].reason exceeds {} UTF-8 bytes",
                sky_dispatch_core::compile::MAX_REASON_BYTES
            )));
        }

        let interned_reason = reason_interns
            .entry(reason.clone())
            .or_insert_with(|| Arc::from(reason))
            .clone();

        actions.push(KeyActionInput {
            source_action_index,
            kind,
            scheduled_us,
            scan_codes,
            reason: interned_reason,
        });
    }
    Ok(actions)
}

pub(super) fn parse_schedule(py_actions: &Bound<'_, PyAny>) -> PyResult<RuntimeSchedule> {
    let actions = parse_actions(py_actions)?;
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(
        &actions,
        &PHYSICAL_INSTRUMENT_SCAN_CODES,
    )
    .map_err(|error| PyValueError::new_err(error.to_string()))?;
    Ok(schedule)
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn parse_schedule_with_allowlist(
    py_actions: &Bound<'_, PyAny>,
    allowed_scan_codes: &Bound<'_, PyAny>,
) -> PyResult<(RuntimeSchedule, Vec<u16>)> {
    let allowed_scan_codes = parse_allowed_scan_codes(allowed_scan_codes)?;
    let actions = parse_actions_with_allowlist(py_actions, &allowed_scan_codes)?;
    let schedule =
        sky_dispatch_core::compile::compile_runtime_intents(&actions, &allowed_scan_codes)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
    Ok((schedule, allowed_scan_codes))
}

pub(super) fn validate_schedule_timing(
    schedule: &RuntimeSchedule,
    effective_min_hold_us: u64,
) -> PyResult<()> {
    sky_dispatch_core::validation::validate_min_hold_feasibility(schedule, effective_min_hold_us)
        .map_err(|error| PyValueError::new_err(error.to_string()))
}
