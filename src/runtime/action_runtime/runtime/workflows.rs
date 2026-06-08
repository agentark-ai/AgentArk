use super::super::*;

impl ActionRuntime {
    /// Execute a workflow action with LLM orchestration
    /// This performs web searches based on the workflow, then passes everything to the LLM
    pub async fn execute_workflow_action(
        &self,
        action_name: &str,
        workflow_content: &str,
        user_query: &str,
        llm: &crate::core::LlmClient,
    ) -> Result<String> {
        tracing::info!("Executing LLM-driven workflow action: {}", action_name);

        // Step 1: Extract search queries from the workflow
        let search_queries = self.extract_search_queries(workflow_content, action_name, user_query);

        // Step 2: Perform web searches
        let mut search_results = Vec::new();
        let search_config = build_search_config(&self.config_dir, self.storage.as_ref()).await;

        for query in &search_queries {
            tracing::debug!("Searching: {}", query);
            let args = crate::actions::search::SearchArgs {
                query: query.clone(),
                num_results: 5,
                backend: None,
                time_scope: None,
            };
            match crate::actions::search::execute_search(&args, &search_config).await {
                Ok(results) => {
                    search_results.push(format!("### Search: {}\n{}", query, results));
                }
                Err(e) => {
                    tracing::warn!("Search failed for '{}': {}", query, e);
                    search_results.push(format!("### Search: {} (failed: {})", query, e));
                }
            }
        }

        // Step 3: Build the LLM prompt with workflow instructions and search results
        let combined_results = search_results.join("\n\n");

        let system_prompt = format!(
            r#"You are executing an action workflow. Your task is to analyze the search results and produce output that EXACTLY follows the output format specified in the workflow.

## ACTION WORKFLOW INSTRUCTIONS
{}

## IMPORTANT RULES
1. Follow the "Output Format" section EXACTLY - use the same structure, headings, and formatting
2. Fill in all placeholder sections with actual content based on the search results or user-supplied evidence
3. Respect any length, style, audience, evidence, or format constraints specified in the workflow
4. Include real data, trends, and insights from the available evidence
5. If evidence is insufficient, note this but still produce the best output possible
6. Use today's date where [Date] is specified: {}

## SEARCH RESULTS OR SUPPORTING EVIDENCE TO ANALYZE
{}
"#,
            workflow_content,
            chrono::Utc::now().format("%Y-%m-%d"),
            combined_results
        );

        let user_prompt = format!(
            "Execute the workflow above. User's additional context/query: '{}'. Generate the complete output following the exact format specified in the workflow.",
            if user_query.is_empty() {
                "none"
            } else {
                user_query
            }
        );

        // Step 4: Call LLM to generate the formatted output
        let response = llm
            .chat(
                &system_prompt,
                &user_prompt,
                &[], // No memory entries needed
                &[], // No additional tools
            )
            .await?;

        Ok(response.content)
    }

    pub(in crate::runtime) fn build_workflow_user_query(arguments: &serde_json::Value) -> String {
        arguments
            .get("query")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                // Backward compatibility: if no explicit "query" exists, pass structured
                // arguments through as JSON so workflow actions can still consume fields.
                if let Some(obj) = arguments.as_object() {
                    if !obj.is_empty() {
                        return serde_json::to_string(arguments).unwrap_or_default();
                    }
                }
                String::new()
            })
    }

    pub(in crate::runtime) fn collect_required_fields_from_schema(
        schema: &serde_json::Value,
    ) -> Vec<String> {
        schema
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    pub(in crate::runtime) fn collect_sensitive_required_fields_from_schema(
        schema: &serde_json::Value,
    ) -> Vec<String> {
        let required = Self::collect_required_fields_from_schema(schema);
        let properties = schema.get("properties").and_then(|value| value.as_object());
        required
            .into_iter()
            .filter(|key| {
                properties
                    .and_then(|items| items.get(key))
                    .and_then(|value| value.as_object())
                    .is_some_and(|property| {
                        property.get("sensitive").and_then(|value| value.as_bool()) == Some(true)
                            || property.get("writeOnly").and_then(|value| value.as_bool())
                                == Some(true)
                            || property
                                .get("format")
                                .and_then(|value| value.as_str())
                                .is_some_and(|value| value.eq_ignore_ascii_case("password"))
                    })
            })
            .collect()
    }

    pub(in crate::runtime) fn has_non_empty_argument(
        arguments: &serde_json::Value,
        key: &str,
    ) -> bool {
        let Some(value) = arguments.get(key) else {
            return false;
        };
        match value {
            serde_json::Value::Null => false,
            serde_json::Value::String(s) => !s.trim().is_empty(),
            serde_json::Value::Array(items) => !items.is_empty(),
            serde_json::Value::Object(map) => !map.is_empty(),
            _ => true,
        }
    }

    pub(in crate::runtime) fn collect_provided_argument_keys(
        arguments: &serde_json::Value,
    ) -> Vec<String> {
        let Some(obj) = arguments.as_object() else {
            return Vec::new();
        };
        obj.iter()
            .filter(|(_, v)| match v {
                serde_json::Value::Null => false,
                serde_json::Value::String(s) => !s.trim().is_empty(),
                serde_json::Value::Array(items) => !items.is_empty(),
                serde_json::Value::Object(map) => !map.is_empty(),
                _ => true,
            })
            .map(|(k, _)| k.to_string())
            .collect()
    }

    pub(in crate::runtime) fn build_workflow_missing_inputs_marker(
        payload: &WorkflowMissingInputsPayload,
    ) -> String {
        let json = serde_json::to_string(payload).unwrap_or_else(|_| {
            let fallback = serde_json::json!({
                "action": payload.action,
                "missing": payload.missing,
                "sensitive_missing": payload.sensitive_missing,
                "required": payload.required,
                "provided": payload.provided,
                "query": payload.query
            });
            fallback.to_string()
        });
        format!("{}{}", WORKFLOW_MISSING_INPUTS_MARKER, json)
    }

    pub(in crate::runtime) fn dedupe_non_empty<I>(items: I) -> Vec<String>
    where
        I: IntoIterator<Item = String>,
    {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for item in items {
            let cleaned = item
                .split('#')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches('`')
                .trim_matches('"')
                .trim_matches('\'')
                .trim_end_matches(',')
                .to_string();
            if cleaned.is_empty() {
                continue;
            }
            let key = cleaned.to_ascii_lowercase();
            if seen.insert(key) {
                out.push(cleaned);
            }
        }
        out
    }

    pub(in crate::runtime) fn slugify_name(value: &str) -> String {
        let mut slug = String::new();
        let mut last_was_separator = false;
        for ch in value.trim().chars() {
            if ch.is_ascii_alphanumeric() {
                slug.push(ch.to_ascii_lowercase());
                last_was_separator = false;
            } else if !last_was_separator {
                slug.push('-');
                last_was_separator = true;
            }
        }
        while slug.ends_with('-') {
            slug.pop();
        }
        slug
    }

    pub(in crate::runtime) fn parse_required_fields_from_frontmatter(
        frontmatter: &str,
    ) -> Vec<String> {
        let mut required = Vec::new();
        let lines: Vec<&str> = frontmatter.lines().collect();
        let mut i = 0usize;

        while i < lines.len() {
            let raw = lines[i];
            let line = raw.trim();
            let is_required_key = line
                .split_once(':')
                .map(|(key, _)| Self::slugify_name(key).replace('-', "_"))
                .is_some_and(|key| {
                    matches!(
                        key.as_str(),
                        "required" | "required_inputs" | "requiredinputs" | "required_fields"
                    )
                });
            if is_required_key {
                let rhs = line
                    .split_once(':')
                    .map(|(_, rhs)| rhs.trim())
                    .unwrap_or("");
                if rhs.starts_with('[') && rhs.ends_with(']') {
                    let inner = &rhs[1..rhs.len().saturating_sub(1)];
                    for part in inner.split(',') {
                        required.push(part.trim().trim_matches('"').trim_matches('\'').to_string());
                    }
                } else if !rhs.is_empty() {
                    required.push(rhs.trim_matches('"').trim_matches('\'').to_string());
                }

                let mut j = i + 1;
                while j < lines.len() {
                    let next_raw = lines[j];
                    let next_trim = next_raw.trim();
                    if next_trim.starts_with("- ") {
                        required.push(
                            next_trim
                                .trim_start_matches("- ")
                                .trim()
                                .trim_matches('"')
                                .trim_matches('\'')
                                .to_string(),
                        );
                        j += 1;
                        continue;
                    }
                    if next_raw.starts_with(' ')
                        || next_raw.starts_with('\t')
                        || next_trim.is_empty()
                    {
                        j += 1;
                        continue;
                    }
                    break;
                }
                i = j;
                continue;
            }
            i += 1;
        }

        Self::dedupe_non_empty(required)
    }

    pub(in crate::runtime) fn parse_required_fields_from_workflow(workflow: &str) -> Vec<String> {
        let mut required = Vec::new();
        let mut in_required_section = false;

        for raw_line in workflow.lines() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }

            if line.starts_with('#') {
                let heading = Self::slugify_name(line.trim_start_matches('#').trim());
                in_required_section = matches!(
                    heading.as_str(),
                    "required-inputs"
                        | "inputs-required"
                        | "required-fields"
                        | "required"
                        | "input-contract"
                );
                continue;
            }

            if line
                .split_once(':')
                .map(|(key, _)| Self::slugify_name(key))
                .is_some_and(|key| matches!(key.as_str(), "required-inputs" | "inputs-required"))
            {
                in_required_section = true;
                continue;
            }

            if in_required_section
                && !line.starts_with("- ")
                && !line.starts_with("* ")
                && line.ends_with(':')
            {
                in_required_section = false;
                continue;
            }

            if !in_required_section {
                continue;
            }

            let candidate = if line.starts_with("- ") {
                line.trim_start_matches("- ").trim()
            } else if let Some(rest) = line.strip_prefix("* ") {
                rest.trim()
            } else {
                continue;
            };

            let mut field =
                if let (Some(start), Some(end)) = (candidate.find('`'), candidate.rfind('`')) {
                    if end > start {
                        candidate[start + 1..end].trim().to_string()
                    } else {
                        candidate.to_string()
                    }
                } else {
                    candidate.to_string()
                };

            if let Some((left, _)) = field.split_once(':') {
                field = left.trim().to_string();
            }
            field = field
                .trim_matches('{')
                .trim_matches('}')
                .trim_matches('[')
                .trim_matches(']')
                .trim()
                .to_string();

            if field
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                required.push(field);
            }
        }

        Self::dedupe_non_empty(required)
    }

    pub(in crate::runtime) fn build_workflow_input_schema(
        frontmatter: &str,
        workflow_content: &str,
    ) -> serde_json::Value {
        let mut required = Self::parse_required_fields_from_frontmatter(frontmatter);
        if required.is_empty() {
            required = Self::parse_required_fields_from_workflow(workflow_content);
        }
        let required = Self::dedupe_non_empty(required);

        let mut properties = serde_json::Map::new();
        properties.insert(
            "query".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Optional free-form input/context for the action"
            }),
        );

        for key in &required {
            if key.eq_ignore_ascii_case("query") {
                continue;
            }
            properties.insert(
                key.clone(),
                serde_json::json!({
                    "type": "string",
                    "description": format!("Required input: {}", key)
                }),
            );
        }

        serde_json::json!({
            "type": "object",
            "properties": properties,
            "required": required
        })
    }

    /// Extract search queries from workflow content based on action type
    pub(in crate::runtime) fn extract_search_queries(
        &self,
        workflow: &str,
        action_name: &str,
        user_query: &str,
    ) -> Vec<String> {
        let mut queries = Vec::new();
        let year = chrono::Utc::now().format("%Y");
        let month = chrono::Utc::now().format("%B");
        let search_disabled = workflow.lines().any(|line| {
            let normalized = line.trim().to_ascii_lowercase();
            normalized == "search: none" || normalized == "search: false"
        });

        // Look for search queries in the workflow (lines starting with - "...")
        for line in workflow.lines() {
            let line = line.trim();
            if line.starts_with("- \"") && line.ends_with("\"") {
                let query = line.trim_start_matches("- \"").trim_end_matches("\"");
                // Replace placeholders
                let query = query
                    .replace("2026", &year.to_string())
                    .replace("February", &month.to_string());
                queries.push(query.to_string());
            }
        }

        if search_disabled {
            return queries;
        }

        // If the workflow does not declare explicit queries, fall back to a
        // generic topic-based set rather than hardcoding skill names here.
        if queries.is_empty() {
            let topic = if user_query.trim().is_empty() {
                action_name.replace('-', " ")
            } else {
                user_query.trim().to_string()
            };
            queries.push(format!("{} latest news {}", topic, year));
            queries.push(format!("{} trends analysis {}", topic, year));
            if !user_query.trim().is_empty() {
                queries.push(format!("{} {}", user_query.trim(), year));
            }
        }

        queries
    }
}
