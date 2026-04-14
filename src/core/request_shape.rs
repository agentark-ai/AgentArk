use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RequestShapeAssessment {
    pub shape: String,
    pub execution_mode: String,
    pub confidence: f32,
    pub should_confirm: bool,
    pub confirmation_question: Option<String>,
    pub reasoning: String,
    pub preferred_actions: Vec<String>,
    pub integration_id: Option<String>,
    pub product_help: bool,
    pub help_topics: Vec<String>,
}

impl RequestShapeAssessment {
    fn canonical_label(value: &str) -> String {
        let mut out = String::new();
        let mut last_was_separator = false;
        for ch in value.trim().to_ascii_lowercase().chars() {
            if ch.is_ascii_alphanumeric() {
                out.push(ch);
                last_was_separator = false;
            } else if !out.is_empty() && !last_was_separator {
                out.push('_');
                last_was_separator = true;
            }
        }
        while out.ends_with('_') {
            out.pop();
        }
        out
    }

    fn normalized_shape(&self) -> String {
        match Self::canonical_label(&self.shape).as_str() {
            "chat" | "convo" | "conversation_like" | "talk" => "conversation".to_string(),
            "inspect" | "inspection_like" | "lookup" | "status" | "question" => {
                "inspection".to_string()
            }
            "automation" | "automated_task" | "scheduled_task" | "reminder" => "task".to_string(),
            "watch" | "monitor" | "monitoring" | "watch_until" => "watcher".to_string(),
            "application" | "website" | "site" | "web_app" | "deployed_app" => "app".to_string(),
            "calendar" | "calendar_event" | "meeting" | "event" | "appointment" => {
                "calendar_event".to_string()
            }
            "gws" | "google_workspace" | "workspace" => "integration".to_string(),
            other => other.to_string(),
        }
    }

    fn normalized_execution_mode(&self) -> String {
        match Self::canonical_label(&self.execution_mode).as_str() {
            "now" | "right_now" | "execute_now" | "run_now" | "direct" => "immediate".to_string(),
            "schedule" | "scheduled_task" | "recurring" | "recurring_schedule" => {
                "scheduled".to_string()
            }
            "watch" | "monitor" | "monitoring" | "until" | "poll_until" => {
                "watch_until".to_string()
            }
            "none_needed" | "no_execution" | "not_applicable" | "na" => "none".to_string(),
            other => other.to_string(),
        }
    }

    pub fn shape_is(&self, expected: &str) -> bool {
        self.normalized_shape()
            == RequestShapeAssessment {
                shape: expected.to_string(),
                ..Default::default()
            }
            .normalized_shape()
    }

    pub fn execution_mode_is(&self, expected: &str) -> bool {
        self.normalized_execution_mode()
            == RequestShapeAssessment {
                execution_mode: expected.to_string(),
                ..Default::default()
            }
            .normalized_execution_mode()
    }

    pub fn is_integration_request(&self) -> bool {
        self.shape_is("integration")
    }

    pub fn is_execution_request(&self) -> bool {
        let shape = self.normalized_shape();
        let execution_mode = self.normalized_execution_mode();
        !matches!(
            shape.as_str(),
            "" | "conversation" | "inspection" | "unknown"
        ) || !matches!(execution_mode.as_str(), "" | "none" | "unknown")
    }

    pub fn is_conversation_like(&self) -> bool {
        if self.is_execution_request() {
            return false;
        }

        matches!(
            self.normalized_shape().as_str(),
            "conversation" | "inspection" | "unknown"
        ) || self.execution_mode_is("none")
    }

    pub fn is_status_like(&self) -> bool {
        self.shape_is("inspection") || self.execution_mode_is("none")
    }
}

#[cfg(test)]
mod tests {
    use super::RequestShapeAssessment;

    #[test]
    fn request_shape_normalizes_schema_aliases() {
        let shape = RequestShapeAssessment {
            shape: "calendar event".to_string(),
            execution_mode: "run now".to_string(),
            ..Default::default()
        };

        assert!(shape.shape_is("calendar_event"));
        assert!(shape.execution_mode_is("immediate"));
        assert!(shape.is_execution_request());
    }
}
