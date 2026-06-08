use super::super::*;

impl Storage {
    // ==================== Swarm Agents ====================

    /// Insert a swarm agent
    pub async fn insert_swarm_agent(&self, agent: &swarm_agent::Model) -> Result<()> {
        swarm_agent::ActiveModel {
            id: Set(agent.id.clone()),
            name: Set(agent.name.clone()),
            agent_type: Set(agent.agent_type.clone()),
            llm_provider: Set(agent.llm_provider.clone()),
            capabilities: Set(agent.capabilities.clone()),
            system_prompt: Set(agent.system_prompt.clone()),
            access_scope: Set(agent.access_scope.clone()),
            enabled: Set(agent.enabled),
            created_at: Set(agent.created_at.clone()),
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    pub async fn upsert_swarm_agent(&self, agent: &swarm_agent::Model) -> Result<()> {
        swarm_agent::Entity::insert(swarm_agent::ActiveModel {
            id: Set(agent.id.clone()),
            name: Set(agent.name.clone()),
            agent_type: Set(agent.agent_type.clone()),
            llm_provider: Set(agent.llm_provider.clone()),
            capabilities: Set(agent.capabilities.clone()),
            system_prompt: Set(agent.system_prompt.clone()),
            access_scope: Set(agent.access_scope.clone()),
            enabled: Set(agent.enabled),
            created_at: Set(agent.created_at.clone()),
        })
        .on_conflict(
            OnConflict::column(swarm_agent::Column::Id)
                .update_columns([
                    swarm_agent::Column::Name,
                    swarm_agent::Column::AgentType,
                    swarm_agent::Column::LlmProvider,
                    swarm_agent::Column::Capabilities,
                    swarm_agent::Column::SystemPrompt,
                    swarm_agent::Column::AccessScope,
                    swarm_agent::Column::Enabled,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    /// Get all swarm agents
    pub async fn get_swarm_agents(&self) -> Result<Vec<swarm_agent::Model>> {
        let agents = swarm_agent::Entity::find().all(&self.db).await?;
        Ok(agents)
    }

    /// Update a persisted swarm agent
    pub async fn update_swarm_agent(&self, agent: &swarm_agent::Model) -> Result<()> {
        swarm_agent::ActiveModel {
            id: Unchanged(agent.id.clone()),
            name: Set(agent.name.clone()),
            agent_type: Set(agent.agent_type.clone()),
            llm_provider: Set(agent.llm_provider.clone()),
            capabilities: Set(agent.capabilities.clone()),
            system_prompt: Set(agent.system_prompt.clone()),
            access_scope: Set(agent.access_scope.clone()),
            enabled: Set(agent.enabled),
            created_at: Set(agent.created_at.clone()),
        }
        .update(&self.db)
        .await?;
        Ok(())
    }

    /// Delete a swarm agent
    pub async fn delete_swarm_agent(&self, id: &str) -> Result<()> {
        swarm_agent::Entity::delete_by_id(id.to_string())
            .exec(&self.db)
            .await?;
        Ok(())
    }

    /// Seed default specialist agents if none exist (first-run only)
    pub async fn seed_default_agents(&self) -> Result<()> {
        let existing = self.get_swarm_agents().await?;
        if !existing.is_empty() {
            return Ok(()); // Already have agents, skip seeding
        }

        tracing::info!("Seeding default specialist agents...");
        let now = chrono::Utc::now().to_rfc3339();

        let defaults = vec![
            swarm_agent::Model {
                id: format!("default-researcher-{}", &uuid::Uuid::new_v4().to_string()[..8]),
                name: "Researcher".to_string(),
                agent_type: "researcher".to_string(),
                llm_provider: "{}".to_string(),
                capabilities: r#"["deep research","web search","data analysis","fact checking","academic research"]"#.to_string(),
                system_prompt: Some("You are a thorough research specialist. When given a topic, search the web, gather multiple sources, cross-reference facts, and present a well-structured summary with key findings, sources, and confidence levels. Be objective and cite your sources.".to_string()),
                access_scope: "{}".to_string(),
                enabled: 1,
                created_at: now.clone(),
            },
            swarm_agent::Model {
                id: format!("default-coder-{}", &uuid::Uuid::new_v4().to_string()[..8]),
                name: "Coder".to_string(),
                agent_type: "coder".to_string(),
                llm_provider: "{}".to_string(),
                capabilities: r#"["code generation","debugging","code review","refactoring","architecture"]"#.to_string(),
                system_prompt: Some("You are an expert software engineer. Write clean, efficient, well-documented code. When debugging, systematically identify root causes. When reviewing code, focus on correctness, performance, security, and maintainability. Support all major programming languages.".to_string()),
                access_scope: "{}".to_string(),
                enabled: 1,
                created_at: now.clone(),
            },
            swarm_agent::Model {
                id: format!("default-writer-{}", &uuid::Uuid::new_v4().to_string()[..8]),
                name: "Writer".to_string(),
                agent_type: "writer".to_string(),
                llm_provider: "{}".to_string(),
                capabilities: r#"["content writing","editing","summarization","translation","creative writing"]"#.to_string(),
                access_scope: "{}".to_string(),
                system_prompt: Some("You are a skilled writer and editor. Adapt your style to the requested format â€” professional emails, blog posts, reports, creative fiction, marketing copy, etc. Focus on clarity, engagement, and proper structure. When editing, preserve the author's voice while improving quality.".to_string()),
                enabled: 1,
                created_at: now.clone(),
            },
            swarm_agent::Model {
                id: format!("default-analyst-{}", &uuid::Uuid::new_v4().to_string()[..8]),
                name: "Analyst".to_string(),
                agent_type: "analyst".to_string(),
                llm_provider: "{}".to_string(),
                capabilities: r#"["data analysis","market research","financial analysis","trend analysis","reporting"]"#.to_string(),
                system_prompt: Some("You are a sharp data and business analyst. Break down complex data, identify patterns and trends, provide actionable insights, and present findings clearly with charts and tables when appropriate. Always quantify your conclusions and flag uncertainties.".to_string()),
                access_scope: "{}".to_string(),
                enabled: 1,
                created_at: now.clone(),
            },
            swarm_agent::Model {
                id: format!("default-planner-{}", &uuid::Uuid::new_v4().to_string()[..8]),
                name: "Planner".to_string(),
                agent_type: "planner".to_string(),
                llm_provider: "{}".to_string(),
                capabilities: r#"["project planning","task breakdown","scheduling","goal setting","strategy"]"#.to_string(),
                system_prompt: Some("You are a strategic planner and project manager. Break down goals into actionable steps, estimate effort, identify dependencies and risks, and create clear timelines. Prioritize using impact vs effort. Always suggest concrete next actions.".to_string()),
                access_scope: "{}".to_string(),
                enabled: 1,
                created_at: now.clone(),
            },
        ];

        for agent in &defaults {
            if let Err(e) = self.insert_swarm_agent(agent).await {
                tracing::warn!("Failed to seed agent '{}': {}", agent.name, e);
            }
        }

        tracing::info!("Seeded {} default specialist agents", defaults.len());
        Ok(())
    }

    // ==================== Swarm Delegations ====================

    /// Get recent swarm delegations
    pub async fn get_recent_delegations(&self, limit: u64) -> Result<Vec<swarm_delegation::Model>> {
        let mut delegations = swarm_delegation::Entity::find()
            .order_by_desc(swarm_delegation::Column::CreatedAt)
            .limit(Self::db_limit(
                limit.min(Self::MAX_SWARM_DELEGATION_ROWS_PER_QUERY),
            ))
            .all(&self.db)
            .await?;
        for delegation in &mut delegations {
            decrypt_swarm_delegation_model(delegation);
        }
        Ok(delegations)
    }

    /// Get all swarm delegations
    pub async fn get_all_delegations(&self) -> Result<Vec<swarm_delegation::Model>> {
        let mut delegations = swarm_delegation::Entity::find()
            .order_by_desc(swarm_delegation::Column::CreatedAt)
            .limit(Self::MAX_SWARM_DELEGATION_ROWS_PER_QUERY)
            .all(&self.db)
            .await?;
        for delegation in &mut delegations {
            decrypt_swarm_delegation_model(delegation);
        }
        Ok(delegations)
    }

    pub async fn get_swarm_delegations_for_parent(
        &self,
        parent_task_id: &str,
    ) -> Result<Vec<swarm_delegation::Model>> {
        let row_id_prefix = format!("{parent_task_id}::");
        let mut delegations = swarm_delegation::Entity::find()
            .filter(
                Condition::any()
                    .add(swarm_delegation::Column::ParentTaskId.eq(parent_task_id.to_string()))
                    .add(swarm_delegation::Column::Id.starts_with(row_id_prefix)),
            )
            .order_by_asc(swarm_delegation::Column::CreatedAt)
            .limit(Self::MAX_SWARM_DELEGATION_ROWS_PER_QUERY)
            .all(&self.db)
            .await?;
        for delegation in &mut delegations {
            decrypt_swarm_delegation_model(delegation);
        }
        Ok(delegations)
    }

    pub async fn get_active_swarm_delegations(
        &self,
        limit: u64,
    ) -> Result<Vec<swarm_delegation::Model>> {
        let mut delegations = swarm_delegation::Entity::find()
            .filter(swarm_delegation::Column::CompletedAt.is_null())
            .order_by_desc(swarm_delegation::Column::CreatedAt)
            .limit(Self::db_limit(
                limit.min(Self::MAX_SWARM_DELEGATION_ROWS_PER_QUERY),
            ))
            .all(&self.db)
            .await?;
        for delegation in &mut delegations {
            decrypt_swarm_delegation_model(delegation);
        }
        Ok(delegations)
    }

    /// Insert a swarm delegation record
    pub async fn insert_swarm_delegation(
        &self,
        delegation: &swarm_delegation::Model,
    ) -> Result<()> {
        swarm_delegation::ActiveModel {
            id: Set(delegation.id.clone()),
            parent_task_id: Set(delegation.parent_task_id.clone()),
            agent_id: Set(delegation.agent_id.clone()),
            task_description: Set(encrypt_storage_string(&delegation.task_description)?),
            result: Set(encrypt_optional_storage_string(
                delegation.result.as_deref(),
            )?),
            success: Set(delegation.success),
            confidence: Set(delegation.confidence),
            execution_time_ms: Set(delegation.execution_time_ms),
            created_at: Set(delegation.created_at.clone()),
            completed_at: Set(delegation.completed_at.clone()),
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    pub async fn upsert_swarm_delegation(
        &self,
        delegation: &swarm_delegation::Model,
    ) -> Result<()> {
        swarm_delegation::Entity::insert(swarm_delegation::ActiveModel {
            id: Set(delegation.id.clone()),
            parent_task_id: Set(delegation.parent_task_id.clone()),
            agent_id: Set(delegation.agent_id.clone()),
            task_description: Set(encrypt_storage_string(&delegation.task_description)?),
            result: Set(encrypt_optional_storage_string(
                delegation.result.as_deref(),
            )?),
            success: Set(delegation.success),
            confidence: Set(delegation.confidence),
            execution_time_ms: Set(delegation.execution_time_ms),
            created_at: Set(delegation.created_at.clone()),
            completed_at: Set(delegation.completed_at.clone()),
        })
        .on_conflict(
            OnConflict::column(swarm_delegation::Column::Id)
                .update_columns([
                    swarm_delegation::Column::ParentTaskId,
                    swarm_delegation::Column::AgentId,
                    swarm_delegation::Column::TaskDescription,
                    swarm_delegation::Column::Result,
                    swarm_delegation::Column::Success,
                    swarm_delegation::Column::Confidence,
                    swarm_delegation::Column::ExecutionTimeMs,
                    swarm_delegation::Column::CompletedAt,
                ])
                .to_owned(),
        )
        .exec(&self.db)
        .await?;
        Ok(())
    }

    pub async fn mark_swarm_run_interrupted(
        &self,
        parent_task_id: &str,
        summary: &str,
    ) -> Result<u64> {
        let now = chrono::Utc::now().to_rfc3339();
        let row_id_prefix = format!("{parent_task_id}::");
        let rows = swarm_delegation::Entity::find()
            .filter(
                Condition::any()
                    .add(swarm_delegation::Column::ParentTaskId.eq(parent_task_id.to_string()))
                    .add(swarm_delegation::Column::Id.starts_with(row_id_prefix)),
            )
            .filter(swarm_delegation::Column::CompletedAt.is_null())
            .all(&self.db)
            .await?;
        let mut updated = 0_u64;
        for row in rows {
            let mut payload = row
                .result
                .clone()
                .and_then(|raw| {
                    serde_json::from_str::<serde_json::Value>(&decrypt_storage_string(&raw)).ok()
                })
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default();
            payload.insert("status".to_string(), serde_json::json!("interrupted"));
            payload.insert("updated_at".to_string(), serde_json::json!(now.clone()));
            if !summary.trim().is_empty() {
                payload.insert("summary".to_string(), serde_json::json!(summary));
                payload.insert("latest_update".to_string(), serde_json::json!(summary));
            }
            let payload_json = serde_json::Value::Object(payload).to_string();
            swarm_delegation::ActiveModel {
                id: Unchanged(row.id),
                result: Set(encrypt_optional_storage_string(Some(
                    payload_json.as_str(),
                ))?),
                success: Set(0),
                completed_at: Set(Some(now.clone())),
                ..Default::default()
            }
            .update(&self.db)
            .await?;
            updated = updated.saturating_add(1);
        }
        Ok(updated)
    }
}
