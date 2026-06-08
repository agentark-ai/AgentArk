use super::super::*;

impl Storage {
    // ==================== LLM Usage ====================

    /// Insert an LLM usage record for analytics (tokens/cost estimation).
    pub async fn insert_llm_usage(&self, usage: &llm_usage::Model) -> Result<()> {
        llm_usage::ActiveModel {
            id: Set(usage.id.clone()),
            created_at: Set(usage.created_at.clone()),
            provider: Set(usage.provider.clone()),
            model: Set(usage.model.clone()),
            channel: Set(usage.channel.clone()),
            purpose: Set(usage.purpose.clone()),
            prompt_tokens: Set(usage.prompt_tokens),
            completion_tokens: Set(usage.completion_tokens),
            total_tokens: Set(usage.total_tokens),
            cached_prompt_tokens: Set(usage.cached_prompt_tokens),
            cache_creation_prompt_tokens: Set(usage.cache_creation_prompt_tokens),
            estimated: Set(usage.estimated),
            cost_usd: Set(usage.cost_usd),
        }
        .insert(&self.db)
        .await?;
        Ok(())
    }

    /// List LLM usage rows since a given RFC3339 timestamp (ascending).
    #[allow(dead_code)]
    pub async fn list_llm_usage_since(&self, since_rfc3339: &str) -> Result<Vec<llm_usage::Model>> {
        let rows = llm_usage::Entity::find()
            .filter(llm_usage::Column::CreatedAt.gte(since_rfc3339.to_string()))
            .order_by_asc(llm_usage::Column::CreatedAt)
            .limit(Self::MAX_LLM_USAGE_ROWS_PER_QUERY)
            .all(&self.db)
            .await?;
        Ok(rows)
    }

    /// List LLM usage rows inside a bounded time window.
    #[allow(dead_code)]
    pub async fn list_llm_usage_between(
        &self,
        from_rfc3339: &str,
        to_rfc3339: &str,
        limit: u64,
    ) -> Result<Vec<llm_usage::Model>> {
        let rows = llm_usage::Entity::find()
            .filter(llm_usage::Column::CreatedAt.gte(from_rfc3339.to_string()))
            .filter(llm_usage::Column::CreatedAt.lt(to_rfc3339.to_string()))
            .order_by_asc(llm_usage::Column::CreatedAt)
            .limit(Self::db_limit(
                limit.min(Self::MAX_LLM_USAGE_ROWS_PER_QUERY),
            ))
            .all(&self.db)
            .await?;
        Ok(rows)
    }

    /// List LLM usage rows for analytics without silently stopping at one page.
    /// Returns a truncation flag when the server-side safety cap is reached.
    pub async fn list_llm_usage_window_complete(
        &self,
        from_rfc3339: &str,
        to_rfc3339: &str,
    ) -> Result<(Vec<llm_usage::Model>, bool)> {
        let mut rows = Vec::new();
        let mut cursor: Option<(String, String)> = None;
        loop {
            let mut query = llm_usage::Entity::find()
                .filter(llm_usage::Column::CreatedAt.gte(from_rfc3339.to_string()))
                .filter(llm_usage::Column::CreatedAt.lt(to_rfc3339.to_string()));
            if let Some((created_at, id)) = cursor.as_ref() {
                query = query.filter(
                    Condition::any()
                        .add(llm_usage::Column::CreatedAt.gt(created_at.clone()))
                        .add(
                            Condition::all()
                                .add(llm_usage::Column::CreatedAt.eq(created_at.clone()))
                                .add(llm_usage::Column::Id.gt(id.clone())),
                        ),
                );
            }
            let page = query
                .order_by_asc(llm_usage::Column::CreatedAt)
                .order_by_asc(llm_usage::Column::Id)
                .limit(Self::MAX_LLM_USAGE_ROWS_PER_QUERY)
                .all(&self.db)
                .await?;
            if page.is_empty() {
                return Ok((rows, false));
            }
            let page_len = page.len();
            for row in page {
                cursor = Some((row.created_at.clone(), row.id.clone()));
                if rows.len() >= Self::MAX_LLM_USAGE_ANALYTICS_ROWS {
                    return Ok((rows, true));
                }
                rows.push(row);
            }
            if page_len < Self::MAX_LLM_USAGE_ROWS_PER_QUERY as usize {
                return Ok((rows, false));
            }
        }
    }
}
