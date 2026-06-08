use super::super::*;

impl Storage {
    pub async fn get_encrypted(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .get(key)
            .await?
            .map(|value| decrypt_storage_bytes(&value)))
    }

    pub async fn set_encrypted(&self, key: &str, value: &[u8]) -> Result<()> {
        let encrypted = encrypt_storage_bytes(value)?;
        self.set(key, &encrypted).await
    }

    pub async fn save_upload_manifest(&self, manifest: &UploadManifest) -> Result<()> {
        let encoded = serde_json::to_vec(manifest)?;
        self.set_encrypted(&Self::upload_manifest_key(&manifest.id), &encoded)
            .await
    }

    pub async fn load_upload_manifest(&self, id: &str) -> Result<Option<UploadManifest>> {
        let Some(raw) = self.get_encrypted(&Self::upload_manifest_key(id)).await? else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_slice::<UploadManifest>(&raw)?))
    }

    pub async fn reencrypt_sensitive_payloads(
        &self,
        old_key: &KeyManager,
        new_key: &KeyManager,
        encrypted_kv_keys: &[&str],
        lineage_record: Option<(String, Vec<u8>)>,
    ) -> Result<()> {
        let txn = self.db.begin().await?;

        let messages = message::Entity::find().all(&txn).await?;
        for row in messages {
            let plaintext = old_key
                .decrypt_string(&row.content)
                .unwrap_or_else(|_| row.content.clone());
            let encrypted = new_key.encrypt_string(&plaintext)?;
            let tool_calls_json = row
                .tool_calls_json
                .as_deref()
                .map(|value| {
                    let plaintext = old_key
                        .decrypt_string(value)
                        .unwrap_or_else(|_| value.to_string());
                    new_key.encrypt_string(&plaintext)
                })
                .transpose()?;
            let tool_call_id = row
                .tool_call_id
                .as_deref()
                .map(|value| {
                    let plaintext = old_key
                        .decrypt_string(value)
                        .unwrap_or_else(|_| value.to_string());
                    new_key.encrypt_string(&plaintext)
                })
                .transpose()?;
            let provider_message_json = row
                .provider_message_json
                .as_deref()
                .map(|value| {
                    let plaintext = old_key
                        .decrypt_string(value)
                        .unwrap_or_else(|_| value.to_string());
                    new_key.encrypt_string(&plaintext)
                })
                .transpose()?;
            message::ActiveModel {
                id: Unchanged(row.id),
                content: Set(encrypted),
                tool_calls_json: Set(tool_calls_json),
                tool_call_id: Set(tool_call_id),
                provider_message_json: Set(provider_message_json),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let tasks = task::Entity::find().all(&txn).await?;
        for row in tasks {
            let description = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.description)
                    .unwrap_or_else(|_| row.description.clone()),
            )?;
            let arguments = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.arguments)
                    .unwrap_or_else(|_| row.arguments.clone()),
            )?;
            let approval = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.approval)
                    .unwrap_or_else(|_| row.approval.clone()),
            )?;
            let result = row.result.map(|value| {
                let plaintext = old_key
                    .decrypt_string(&value)
                    .unwrap_or_else(|_| value.clone());
                new_key.encrypt_string(&plaintext)
            });
            task::ActiveModel {
                id: Unchanged(row.id),
                description: Set(description),
                arguments: Set(arguments),
                approval: Set(approval),
                result: Set(result.transpose()?),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let approvals = approval_log::Entity::find().all(&txn).await?;
        for row in approvals {
            let plaintext = old_key
                .decrypt_string(&row.arguments)
                .unwrap_or_else(|_| row.arguments.clone());
            let encrypted = new_key.encrypt_string(&plaintext)?;
            approval_log::ActiveModel {
                id: Unchanged(row.id),
                arguments: Set(encrypted),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let traces = execution_trace::Entity::find().all(&txn).await?;
        for row in traces {
            let message = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.message)
                    .unwrap_or_else(|_| row.message.clone()),
            )?;
            let steps_json = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.steps_json)
                    .unwrap_or_else(|_| row.steps_json.clone()),
            )?;
            let response = row.response.map(|value| {
                let plaintext = old_key
                    .decrypt_string(&value)
                    .unwrap_or_else(|_| value.clone());
                new_key.encrypt_string(&plaintext)
            });
            execution_trace::ActiveModel {
                id: Unchanged(row.id),
                message: Set(message),
                steps_json: Set(steps_json),
                response: Set(response.transpose()?),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let user_data_items = user_data_item::Entity::find().all(&txn).await?;
        for row in user_data_items {
            let title = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.title)
                    .unwrap_or_else(|_| row.title.clone()),
            )?;
            let content = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.content)
                    .unwrap_or_else(|_| row.content.clone()),
            )?;
            user_data_item::ActiveModel {
                id: Unchanged(row.id),
                title: Set(title),
                content: Set(content),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let knowledge_items = knowledge_item::Entity::find().all(&txn).await?;
        for row in knowledge_items {
            let title = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.title)
                    .unwrap_or_else(|_| row.title.clone()),
            )?;
            let content = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.content)
                    .unwrap_or_else(|_| row.content.clone()),
            )?;
            knowledge_item::ActiveModel {
                id: Unchanged(row.id),
                title: Set(title),
                content: Set(content),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let experience_items = experience_item::Entity::find().all(&txn).await?;
        for row in experience_items {
            let title = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.title)
                    .unwrap_or_else(|_| row.title.clone()),
            )?;
            let content = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.content)
                    .unwrap_or_else(|_| row.content.clone()),
            )?;
            experience_item::ActiveModel {
                id: Unchanged(row.id),
                title: Set(title),
                content: Set(content),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let user_preferences = user_preference::Entity::find().all(&txn).await?;
        for row in user_preferences {
            let plaintext = old_key
                .decrypt_string(&row.value)
                .unwrap_or_else(|_| row.value.clone());
            let encrypted = new_key.encrypt_string(&plaintext)?;
            user_preference::ActiveModel {
                id: Unchanged(row.id),
                value: Set(encrypted),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let document_chunks = document_chunk::Entity::find().all(&txn).await?;
        for row in document_chunks {
            let plaintext = old_key
                .decrypt_string(&row.content)
                .unwrap_or_else(|_| row.content.clone());
            let encrypted = new_key.encrypt_string(&plaintext)?;
            document_chunk::ActiveModel {
                id: Unchanged(row.id),
                content: Set(encrypted),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let documents = document::Entity::find().all(&txn).await?;
        for row in documents {
            let plaintext = old_key
                .decrypt_string(&row.filename)
                .unwrap_or_else(|_| row.filename.clone());
            let encrypted = new_key.encrypt_string(&plaintext)?;
            document::ActiveModel {
                id: Unchanged(row.id),
                filename: Set(encrypted),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let notifications = notification::Entity::find().all(&txn).await?;
        for row in notifications {
            let title_plaintext = old_key
                .decrypt_string(&row.title)
                .unwrap_or_else(|_| row.title.clone());
            let body_plaintext = old_key
                .decrypt_string(&row.body)
                .unwrap_or_else(|_| row.body.clone());
            let encrypted_title = new_key.encrypt_string(&title_plaintext)?;
            let encrypted_body = new_key.encrypt_string(&body_plaintext)?;
            notification::ActiveModel {
                id: Unchanged(row.id),
                title: Set(encrypted_title),
                body: Set(encrypted_body),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let security_logs = security_log::Entity::find().all(&txn).await?;
        for row in security_logs {
            let message = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.message)
                    .unwrap_or_else(|_| row.message.clone()),
            )?;
            let source = row.source.map(|value| {
                let plaintext = old_key
                    .decrypt_string(&value)
                    .unwrap_or_else(|_| value.clone());
                new_key.encrypt_string(&plaintext)
            });
            security_log::ActiveModel {
                id: Unchanged(row.id),
                message: Set(message),
                source: Set(source.transpose()?),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let operational_logs = operational_log::Entity::find().all(&txn).await?;
        for row in operational_logs {
            let outcome = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.outcome)
                    .unwrap_or_else(|_| row.outcome.clone()),
            )?;
            let arguments = row.arguments.map(|value| {
                let plaintext = old_key
                    .decrypt_string(&value)
                    .unwrap_or_else(|_| value.clone());
                new_key.encrypt_string(&plaintext)
            });
            let payload = row.payload.map(|value| {
                let plaintext = old_key
                    .decrypt_string(&value)
                    .unwrap_or_else(|_| value.clone());
                new_key.encrypt_string(&plaintext)
            });
            operational_log::ActiveModel {
                id: Unchanged(row.id),
                outcome: Set(outcome),
                arguments: Set(arguments.transpose()?),
                payload: Set(payload.transpose()?),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let delegations = swarm_delegation::Entity::find().all(&txn).await?;
        for row in delegations {
            let task_description = new_key.encrypt_string(
                &old_key
                    .decrypt_string(&row.task_description)
                    .unwrap_or_else(|_| row.task_description.clone()),
            )?;
            let result = row.result.map(|value| {
                let plaintext = old_key
                    .decrypt_string(&value)
                    .unwrap_or_else(|_| value.clone());
                new_key.encrypt_string(&plaintext)
            });
            swarm_delegation::ActiveModel {
                id: Unchanged(row.id),
                task_description: Set(task_description),
                result: Set(result.transpose()?),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let automation_runs = automation_run::Entity::find().all(&txn).await?;
        for row in automation_runs {
            let plaintext = old_key
                .decrypt_string(&row.payload)
                .unwrap_or_else(|_| row.payload.clone());
            let encrypted = new_key.encrypt_string(&plaintext)?;
            automation_run::ActiveModel {
                id: Unchanged(row.id),
                payload: Set(encrypted),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        let automation_states = automation_supervisor_state::Entity::find()
            .all(&txn)
            .await?;
        for row in automation_states {
            let plaintext = old_key
                .decrypt_string(&row.payload)
                .unwrap_or_else(|_| row.payload.clone());
            let encrypted = new_key.encrypt_string(&plaintext)?;
            automation_supervisor_state::ActiveModel {
                automation_id: Unchanged(row.automation_id),
                payload: Set(encrypted),
                ..Default::default()
            }
            .update(&txn)
            .await?;
        }

        if !encrypted_kv_keys.is_empty() {
            let keys = encrypted_kv_keys
                .iter()
                .map(|key| (*key).to_string())
                .collect::<Vec<_>>();
            let rows = kv_store::Entity::find()
                .filter(kv_store::Column::Key.is_in(keys))
                .all(&txn)
                .await?;
            let now = chrono::Utc::now().to_rfc3339();
            for row in rows {
                let plaintext = old_key
                    .decrypt(&row.value)
                    .unwrap_or_else(|_| row.value.clone());
                let encrypted = new_key.encrypt(&plaintext)?;
                kv_store::ActiveModel {
                    key: Unchanged(row.key),
                    value: Set(encrypted),
                    updated_at: Set(now.clone()),
                    ..Default::default()
                }
                .update(&txn)
                .await?;
            }
        }

        if let Some((lineage_key, lineage_value)) = lineage_record {
            let now = chrono::Utc::now().to_rfc3339();
            kv_store::Entity::insert(kv_store::ActiveModel {
                key: Set(lineage_key),
                value: Set(lineage_value),
                created_at: Set(now.clone()),
                updated_at: Set(now),
            })
            .on_conflict(
                OnConflict::column(kv_store::Column::Key)
                    .update_columns([kv_store::Column::Value, kv_store::Column::UpdatedAt])
                    .to_owned(),
            )
            .exec(&txn)
            .await?;
        }

        txn.commit().await?;
        Ok(())
    }

    pub async fn ensure_sensitive_payloads_encrypted(
        &self,
        key_manager: &KeyManager,
        encrypted_kv_keys: &[&str],
    ) -> Result<bool> {
        let already_backfilled = self
            .get(Self::SENSITIVE_PAYLOAD_BACKFILL_MARKER_KEY)
            .await?
            .map(|bytes| bytes == b"done")
            .unwrap_or(false);
        if already_backfilled {
            return Ok(false);
        }

        let lineage_record = serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "fingerprint": key_manager.fingerprint(),
            "recorded_at": chrono::Utc::now().to_rfc3339(),
        }))?;
        self.reencrypt_sensitive_payloads(
            key_manager,
            key_manager,
            encrypted_kv_keys,
            Some((
                crate::core::runtime::config::SETTINGS_KEY_LINEAGE_KEY.to_string(),
                lineage_record,
            )),
        )
        .await?;
        self.set(Self::SENSITIVE_PAYLOAD_BACKFILL_MARKER_KEY, b"done")
            .await?;
        Ok(true)
    }
}
