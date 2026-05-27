use anyhow::Result;
use serde_json::Value;

use crate::core::arkorbit::{validate_writable_orbit_path, ArkOrbitService};

use super::validators::require_string;

pub async fn orbit_file_write(service: &ArkOrbitService, args: &Value) -> Result<String> {
    let orbit_id = require_string(args, "orbit_id")?;
    let path = require_string(args, "path")?;
    validate_writable_orbit_path(path)?;
    let content = args
        .get("content")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    service.write_orbit_file(orbit_id, path, content)?;
    Ok(serde_json::to_string(&serde_json::json!({
        "status": "written",
        "orbit_id": orbit_id,
        "path": path,
        "bytes": content.len(),
    }))?)
}
