//! Unit tests for the swarm system

#[cfg(test)]
mod swarm_tests {
    use crate::core::llm::LlmProvider;
    use crate::core::orchestra::SubAgentType;
    use crate::core::swarm::agent_trait::*;
    use crate::core::swarm::bus::*;
    use crate::core::swarm::coordinator::*;
    use crate::core::swarm::messages::*;
    use crate::core::swarm::registry::*;
    use crate::core::swarm::specialist::*;

    // ==================== AgentId ====================

    #[test]
    fn test_agent_id_uniqueness() {
        let id1 = AgentId::new();
        let id2 = AgentId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_agent_id_clone_eq() {
        let id = AgentId::new();
        assert_eq!(id, id.clone());
    }

    #[test]
    fn test_agent_id_display() {
        let id = AgentId::new();
        let s = format!("{}", id);
        assert!(uuid::Uuid::parse_str(&s).is_ok());
    }

    #[test]
    fn test_agent_id_hash_map() {
        use std::collections::HashMap;
        let id = AgentId::new();
        let mut map = HashMap::new();
        map.insert(id.clone(), 42);
        assert_eq!(map[&id], 42);
    }

    // ==================== AgentCapability ====================

    #[test]
    fn test_capability_serialization() {
        let cap = AgentCapability {
            name: "Test".into(),
            description: "Desc".into(),
            keywords: vec!["a".into(), "b".into()],
        };
        let json = serde_json::to_string(&cap).unwrap();
        let de: AgentCapability = serde_json::from_str(&json).unwrap();
        assert_eq!(de.name, "Test");
        assert_eq!(de.keywords.len(), 2);
    }

    // ==================== Registry ====================

    #[tokio::test]
    async fn test_registry_register_count() {
        let reg = AgentRegistry::new();
        assert_eq!(reg.count().await, 0);
        reg.register(make_info("A")).await;
        assert_eq!(reg.count().await, 1);
        reg.register(make_info("B")).await;
        assert_eq!(reg.count().await, 2);
    }

    #[tokio::test]
    async fn test_registry_unregister() {
        let reg = AgentRegistry::new();
        let info = make_info("X");
        let id = info.id.clone();
        reg.register(info).await;
        assert_eq!(reg.count().await, 1);
        reg.unregister(&id).await;
        assert_eq!(reg.count().await, 0);
    }

    #[tokio::test]
    async fn test_registry_get() {
        let reg = AgentRegistry::new();
        let info = make_info("FindMe");
        let id = info.id.clone();
        reg.register(info).await;
        assert!(reg.get(&id).await.is_some());
        assert!(reg.get(&AgentId::new()).await.is_none());
    }

    #[tokio::test]
    async fn test_registry_update_status() {
        let reg = AgentRegistry::new();
        let info = make_info("S");
        let id = info.id.clone();
        reg.register(info).await;

        reg.update_status(&id, AgentStatus::Busy).await;
        assert_eq!(reg.get(&id).await.unwrap().status, AgentStatus::Busy);

        reg.update_status(&id, AgentStatus::Idle).await;
        assert_eq!(reg.get(&id).await.unwrap().status, AgentStatus::Idle);
    }

    #[tokio::test]
    async fn test_registry_find_by_capability() {
        let reg = AgentRegistry::new();
        let mut r = make_info("Researcher");
        r.capabilities = vec![AgentCapability {
            name: "search".into(),
            description: "web search".into(),
            keywords: vec!["search".into(), "web".into()],
        }];
        let mut c = make_info("Coder");
        c.capabilities = vec![AgentCapability {
            name: "coding".into(),
            description: "write code".into(),
            keywords: vec!["code".into(), "program".into()],
        }];
        reg.register(r).await;
        reg.register(c).await;

        assert_eq!(reg.find_by_capability("search").await.len(), 1);
        assert_eq!(reg.find_by_capability("code").await.len(), 1);
        assert_eq!(reg.find_by_capability("nope").await.len(), 0);
    }

    #[tokio::test]
    async fn test_registry_active_count() {
        let reg = AgentRegistry::new();
        let i1 = make_info("A");
        let id1 = i1.id.clone();
        let i2 = make_info("B");
        let id2 = i2.id.clone();
        reg.register(i1).await;
        reg.register(i2).await;

        assert_eq!(reg.active_count().await, 0);
        reg.update_status(&id1, AgentStatus::Busy).await;
        assert_eq!(reg.active_count().await, 1);
        reg.update_status(&id2, AgentStatus::Busy).await;
        assert_eq!(reg.active_count().await, 2);
    }

    #[tokio::test]
    async fn test_registry_list() {
        let reg = AgentRegistry::new();
        reg.register(make_info("Alpha")).await;
        reg.register(make_info("Beta")).await;
        let list = reg.list().await;
        assert_eq!(list.len(), 2);
        let names: Vec<_> = list.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"Alpha"));
        assert!(names.contains(&"Beta"));
    }

    // ==================== MessageBus ====================

    #[tokio::test]
    async fn test_bus_send_receive() {
        let bus = MessageBus::new();
        let from = AgentId::new();
        let to = AgentId::new();
        let mut rx = bus.create_mailbox(to.clone()).await;

        let msg = SwarmMessage::new(from, to.clone(), "hello".into());
        bus.send(msg).await.unwrap();

        let received = rx.recv().await.unwrap();
        assert_eq!(received.content, "hello");
    }

    #[tokio::test]
    async fn test_bus_send_to_missing() {
        let bus = MessageBus::new();
        let msg = SwarmMessage::new(AgentId::new(), AgentId::new(), "x".into());
        assert!(bus.send(msg).await.is_err());
    }

    #[tokio::test]
    async fn test_bus_broadcast() {
        let bus = MessageBus::new();
        let mut rx = bus.subscribe_events();
        bus.broadcast(SwarmEvent::AgentRegistered(make_info("Test")));
        let event = rx.recv().await.unwrap();
        match event {
            SwarmEvent::AgentRegistered(info) => assert_eq!(info.name, "Test"),
            _ => panic!("wrong event type"),
        }
    }

    #[tokio::test]
    async fn test_bus_remove_mailbox() {
        let bus = MessageBus::new();
        let id = AgentId::new();
        let _rx = bus.create_mailbox(id.clone()).await;
        bus.remove_mailbox(&id).await;

        let msg = SwarmMessage::new(AgentId::new(), id, "x".into());
        assert!(bus.send(msg).await.is_err());
    }

    #[tokio::test]
    async fn test_bus_multiple_messages() {
        let bus = MessageBus::new();
        let from = AgentId::new();
        let to = AgentId::new();
        let mut rx = bus.create_mailbox(to.clone()).await;

        for i in 0..5 {
            let msg = SwarmMessage::new(from.clone(), to.clone(), format!("msg-{}", i));
            bus.send(msg).await.unwrap();
        }
        for i in 0..5 {
            assert_eq!(rx.recv().await.unwrap().content, format!("msg-{}", i));
        }
    }

    // ==================== SwarmMessage ====================

    #[test]
    fn test_message_creation() {
        let msg = SwarmMessage::new(AgentId::new(), AgentId::new(), "test".into());
        assert_eq!(msg.content, "test");
        assert!(msg.context.is_none());
    }

    #[test]
    fn test_message_serialization() {
        let msg = SwarmMessage::new(AgentId::new(), AgentId::new(), "hi".into());
        let json = serde_json::to_string(&msg).unwrap();
        let de: SwarmMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(de.content, "hi");
    }

    // ==================== DelegationResult ====================

    #[test]
    fn test_delegation_result_success() {
        let r = DelegationResult {
            task_id: uuid::Uuid::new_v4(),
            agent_id: AgentId::new(),
            agent_name: "T".into(),
            success: true,
            content: "done".into(),
            confidence: 0.9,
            execution_time_ms: 100,
            error: None,
        };
        assert!(r.success);
        assert_eq!(r.confidence, 0.9);
    }

    #[test]
    fn test_delegation_result_failure() {
        let r = DelegationResult {
            task_id: uuid::Uuid::new_v4(),
            agent_id: AgentId::new(),
            agent_name: "F".into(),
            success: false,
            content: String::new(),
            confidence: 0.0,
            execution_time_ms: 50,
            error: Some("Timeout".into()),
        };
        assert!(!r.success);
        assert_eq!(r.error.as_deref(), Some("Timeout"));
    }

    // ==================== SwarmConfig ====================

    #[test]
    fn test_config_default() {
        let c = SwarmConfig::default();
        assert_eq!(c.max_specialists, 5);
        assert_eq!(c.default_timeout_secs, 0);
        assert!(c.specialists.is_empty());
    }

    #[test]
    fn test_config_serialization() {
        let c = SwarmConfig {
            max_specialists: 10,
            default_timeout_secs: 120,
            specialists: vec![],
        };
        let json = serde_json::to_string(&c).unwrap();
        let de: SwarmConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(de.max_specialists, 10);
    }

    // ==================== SpecialistAgent ====================

    #[test]
    fn test_specialist_creation() {
        let agent = make_specialist("Coder", SubAgentType::Coder);
        assert_eq!(agent.config().name, "Coder");
        assert_eq!(agent.model_name(), "llama3.2");
    }

    #[test]
    fn test_specialist_can_handle_research_vs_code() {
        let researcher = make_specialist("R", SubAgentType::Researcher);
        let coder = make_specialist("C", SubAgentType::Coder);

        let r_score = researcher.can_handle("research AI trends");
        let c_score = coder.can_handle("research AI trends");
        assert!(
            r_score > c_score,
            "Researcher {} > Coder {} for research",
            r_score,
            c_score
        );

        let c_score2 = coder.can_handle("write a Python script");
        let r_score2 = researcher.can_handle("write a Python script");
        assert!(
            c_score2 > r_score2,
            "Coder {} > Researcher {} for coding",
            c_score2,
            r_score2
        );
    }

    #[test]
    fn test_specialist_can_handle_with_capabilities() {
        let agent = SpecialistAgent::new(
            SpecialistConfig {
                id: None,
                name: "DataExpert".into(),
                agent_type: SubAgentType::Analyst,
                llm_provider: ollama_provider(),
                system_prompt_override: None,
                max_memory_retrieval: 3,
                capabilities: vec![AgentCapability {
                    name: "Data".into(),
                    description: "analysis".into(),
                    keywords: vec!["data".into(), "statistics".into(), "analysis".into()],
                }],
                access_scope: crate::core::swarm::AgentAccessScope::default(),
                enabled: true,
            },
            vec![],
        )
        .unwrap();

        assert!(agent.can_handle("data analysis statistics") > 0.0);
    }

    #[test]
    fn test_specialist_info() {
        let agent = make_specialist("InfoTest", SubAgentType::Writer);
        let info = agent.info();
        assert_eq!(info.name, "InfoTest");
        assert_eq!(info.status, AgentStatus::Idle);
        assert_eq!(info.llm_model, "llama3.2");
    }

    #[test]
    fn test_specialist_model_names() {
        let ollama = make_specialist("O", SubAgentType::Researcher);
        assert_eq!(ollama.model_name(), "llama3.2");

        let anthropic = SpecialistAgent::new(
            SpecialistConfig {
                id: None,
                name: "A".into(),
                agent_type: SubAgentType::Coder,
                llm_provider: LlmProvider::Anthropic {
                    api_key: "k".into(),
                    model: "claude-sonnet-4-20250514".into(),
                },
                system_prompt_override: None,
                max_memory_retrieval: 3,
                capabilities: vec![],
                access_scope: crate::core::swarm::AgentAccessScope::default(),
                enabled: true,
            },
            vec![],
        )
        .unwrap();
        assert_eq!(anthropic.model_name(), "claude-sonnet-4-20250514");

        let openai = SpecialistAgent::new(
            SpecialistConfig {
                id: None,
                name: "G".into(),
                agent_type: SubAgentType::Writer,
                llm_provider: LlmProvider::OpenAI {
                    api_key: "k".into(),
                    model: "gpt-4o".into(),
                    base_url: None,
                },
                system_prompt_override: None,
                max_memory_retrieval: 3,
                capabilities: vec![],
                access_scope: crate::core::swarm::AgentAccessScope::default(),
                enabled: true,
            },
            vec![],
        )
        .unwrap();
        assert_eq!(openai.model_name(), "gpt-4o");
    }

    // ==================== SwarmManager ====================

    #[tokio::test]
    async fn test_manager_creation() {
        let m = SwarmManager::new(SwarmConfig::default()).await.unwrap();
        let s = m.status().await;
        assert!(s.enabled); // Always true — agents auto-spawn
        assert_eq!(s.total_agents, 0);
    }

    #[tokio::test]
    async fn test_manager_add_specialist() {
        let m = SwarmManager::new(SwarmConfig::default()).await.unwrap();
        let id = m
            .add_specialist(
                make_spec_config("TestAgent", SubAgentType::Researcher),
                vec![],
            )
            .await
            .unwrap();
        assert_eq!(m.status().await.total_agents, 1);
        assert!(m.registry.get(&id).await.is_some());
    }

    #[tokio::test]
    async fn test_manager_remove_specialist() {
        let m = SwarmManager::new(SwarmConfig::default()).await.unwrap();
        let id = m
            .add_specialist(make_spec_config("Rm", SubAgentType::Coder), vec![])
            .await
            .unwrap();
        assert_eq!(m.status().await.total_agents, 1);
        m.remove_specialist(&id).await.unwrap();
        assert_eq!(m.status().await.total_agents, 0);
    }

    #[tokio::test]
    async fn test_manager_multiple_agents() {
        let m = SwarmManager::new(SwarmConfig::default()).await.unwrap();
        for (name, t) in [
            ("R", SubAgentType::Researcher),
            ("C", SubAgentType::Coder),
            ("A", SubAgentType::Analyst),
        ] {
            m.add_specialist(make_spec_config(name, t), vec![])
                .await
                .unwrap();
        }
        assert_eq!(m.status().await.total_agents, 3);
        assert_eq!(m.status().await.active_agents, 0);
    }

    #[tokio::test]
    async fn test_manager_delegate_no_agents() {
        let m = SwarmManager::new(SwarmConfig::default()).await.unwrap();
        let llm = crate::core::llm::LlmClient::new(&ollama_provider()).unwrap();
        let result = m.delegate("test", "ctx", &llm, &[], &[], None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No specialist"));
    }

    #[tokio::test]
    async fn test_manager_init_with_specialists() {
        let config = SwarmConfig {
            max_specialists: 5,
            default_timeout_secs: 0,
            specialists: vec![make_spec_config("Auto", SubAgentType::Researcher)],
        };
        let m = SwarmManager::new(config).await.unwrap();
        assert_eq!(m.status().await.total_agents, 1);
    }

    #[tokio::test]
    async fn test_manager_disabled_specialists_skipped() {
        let mut disabled = make_spec_config("Disabled", SubAgentType::Coder);
        disabled.enabled = false;
        let config = SwarmConfig {
            max_specialists: 5,
            default_timeout_secs: 0,
            specialists: vec![
                make_spec_config("Enabled", SubAgentType::Researcher),
                disabled,
            ],
        };
        let m = SwarmManager::new(config).await.unwrap();
        assert_eq!(m.status().await.total_agents, 1);
        assert_eq!(m.status().await.agents[0].name, "Enabled");
    }

    // ==================== SwarmEvent ====================

    #[test]
    fn test_event_serialization() {
        let e1 = SwarmEvent::AgentRegistered(make_info("T"));
        let j1 = serde_json::to_string(&e1).unwrap();
        assert!(j1.contains("AgentRegistered"));

        let e2 = SwarmEvent::TaskCompleted {
            task_id: uuid::Uuid::new_v4(),
            agent_id: AgentId::new(),
            success: true,
        };
        let j2 = serde_json::to_string(&e2).unwrap();
        assert!(j2.contains("TaskCompleted"));
    }

    // ==================== Helpers ====================

    fn ollama_provider() -> LlmProvider {
        LlmProvider::Ollama {
            base_url: "http://localhost:11434".into(),
            model: "llama3.2".into(),
        }
    }

    fn make_info(name: &str) -> AgentInfo {
        AgentInfo {
            id: AgentId::new(),
            name: name.into(),
            agent_type: "Test".into(),
            capabilities: vec![],
            status: AgentStatus::Idle,
            llm_model: "test".into(),
        }
    }

    fn make_specialist(name: &str, agent_type: SubAgentType) -> SpecialistAgent {
        SpecialistAgent::new(make_spec_config(name, agent_type), vec![]).unwrap()
    }

    fn make_spec_config(name: &str, agent_type: SubAgentType) -> SpecialistConfig {
        SpecialistConfig {
            id: None,
            name: name.into(),
            agent_type,
            llm_provider: ollama_provider(),
            system_prompt_override: None,
            max_memory_retrieval: 3,
            capabilities: vec![],
            access_scope: crate::core::swarm::AgentAccessScope::default(),
            enabled: true,
        }
    }
}
