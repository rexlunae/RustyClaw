#[cfg(test)]
mod tests {
    use crate::mnemo::{
        config::SummarizationConfig, 
        generate_context_md,
        DeterministicSummarizer, 
        MemoryEntry, 
        MemoryStore,
        MnemoConfig, 
        SqliteMemoryStore,
    };
    use tempfile::TempDir;

    fn test_config() -> MnemoConfig {
        MnemoConfig {
            enabled: true,
            db_path: None,
            fresh_tail_messages: 3,
            leaf_chunk_size: 4,
            condensed_chunk_size: 2,
            threshold_items: 8,
            summarization: SummarizationConfig::default(),
        }
    }

    #[tokio::test]
    async fn test_store_open() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.sqlite3");
        let config = test_config();

        let store = SqliteMemoryStore::open(&db_path, config).await.unwrap();

        assert_eq!(store.name(), "sqlite");
        assert_eq!(store.message_count().await.unwrap(), 0);
        assert_eq!(store.summary_count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_ingest_message() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.sqlite3");
        let config = test_config();

        let store = SqliteMemoryStore::open(&db_path, config).await.unwrap();

        let msg_id = store.ingest("user", "Hello world", 3).await.unwrap();
        assert!(msg_id > 0);

        assert_eq!(store.message_count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn test_get_context() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.sqlite3");
        let config = test_config();

        let store = SqliteMemoryStore::open(&db_path, config).await.unwrap();

        store.ingest("user", "Hello", 2).await.unwrap();
        store.ingest("assistant", "Hi there!", 3).await.unwrap();
        store.ingest("user", "How are you?", 4).await.unwrap();

        let entries = store.get_context_entries(1000).await.unwrap();
        assert_eq!(entries.len(), 3);

        assert_eq!(entries[0].content, "Hello");
        assert_eq!(entries[1].content, "Hi there!");
        assert_eq!(entries[2].content, "How are you?");
    }

    #[tokio::test]
    async fn test_search() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.sqlite3");
        let config = test_config();

        let store = SqliteMemoryStore::open(&db_path, config).await.unwrap();

        store
            .ingest("user", "I love Rust programming", 6)
            .await
            .unwrap();
        store
            .ingest("user", "Python is also great", 5)
            .await
            .unwrap();
        store.ingest("user", "Rust is fast", 4).await.unwrap();

        let results = store.search("Rust", 10).await.unwrap();
        assert_eq!(results.len(), 2);
        assert!(results[0].entry.content.contains("Rust"));
    }

    #[tokio::test]
    async fn test_compaction_with_deterministic_summarizer() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.sqlite3");
        let config = test_config(); // threshold = 8

        let store = SqliteMemoryStore::open(&db_path, config).await.unwrap();
        let summarizer = DeterministicSummarizer::new(180, 900);

        // Add 10 messages (above threshold of 8)
        for i in 0..10 {
            store
                .ingest("user", &format!("Message number {}", i), 4)
                .await
                .unwrap();
        }

        assert_eq!(store.message_count().await.unwrap(), 10);
        assert_eq!(store.summary_count().await.unwrap(), 0);

        // Run compaction
        let stats = store.compact(&summarizer).await.unwrap();

        assert!(stats.messages_compacted > 0);
        assert_eq!(stats.summaries_created, 1);
        assert_eq!(store.summary_count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn test_generate_context_md() {
        let entries = vec![
            MemoryEntry {
                id: 1,
                role: "user".to_string(),
                content: "Hello world".to_string(),
                token_count: 3,
                timestamp: 0,
                depth: 0,
            },
            MemoryEntry {
                id: 2,
                role: "summary".to_string(),
                content: "A greeting exchange".to_string(),
                token_count: 5,
                timestamp: 0,
                depth: 1,
            },
        ];

        let md = generate_context_md(&entries);
        assert!(md.contains("# MNEMO CONTEXT"));
        assert!(md.contains("## user (msg #1)"));
        assert!(md.contains("Hello world"));
        assert!(md.contains("## Summary d1 #2"));
        assert!(md.contains("A greeting exchange"));
    }
}
