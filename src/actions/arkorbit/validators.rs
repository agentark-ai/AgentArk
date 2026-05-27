//! Shared structural validators for ArkOrbit tools.

use anyhow::{anyhow, Result};
use serde_json::Value;

pub(super) fn require_string<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    let raw = args
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "arkorbit: '{}' is required and must be a non-empty string",
                key
            )
        })?;
    Ok(raw)
}

pub(super) fn optional_string<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}
