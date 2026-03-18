//! SQLite schema for mnemo storage.

/// SQL to create the mnemo database schema.
pub const SCHEMA: &str = r#"
-- Conversations table (agent + session pairs)
CREATE TABLE IF NOT EXISTS conversations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    UNIQUE(agent_id, session_id)
);

-- Messages table (immutable transcript)
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id INTEGER NOT NULL REFERENCES conversations(id),
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    seq INTEGER NOT NULL,
    token_count INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now')),
    UNIQUE(conversation_id, seq)
);

-- Summaries table (compacted content)
CREATE TABLE IF NOT EXISTS summaries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id INTEGER NOT NULL REFERENCES conversations(id),
    depth INTEGER NOT NULL DEFAULT 0,
    content TEXT NOT NULL,
    token_count INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

-- Summary-message links (which messages were summarized)
CREATE TABLE IF NOT EXISTS summary_messages (
    summary_id INTEGER NOT NULL REFERENCES summaries(id),
    message_id INTEGER NOT NULL REFERENCES messages(id),
    PRIMARY KEY (summary_id, message_id)
);

-- Summary-summary links (condensed summary sources, DAG)
CREATE TABLE IF NOT EXISTS summary_sources (
    summary_id INTEGER NOT NULL REFERENCES summaries(id),
    source_summary_id INTEGER NOT NULL REFERENCES summaries(id),
    PRIMARY KEY (summary_id, source_summary_id)
);

-- Context items (active frontier for agent bootstrap)
CREATE TABLE IF NOT EXISTS context_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id INTEGER NOT NULL REFERENCES conversations(id),
    item_type TEXT NOT NULL CHECK (item_type IN ('message', 'summary')),
    ref_id INTEGER NOT NULL,
    position INTEGER NOT NULL,
    UNIQUE(conversation_id, position)
);

-- Compaction events (audit log)
CREATE TABLE IF NOT EXISTS compaction_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id INTEGER NOT NULL REFERENCES conversations(id),
    event_type TEXT NOT NULL,
    source_ids TEXT NOT NULL,
    result_id INTEGER,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

-- Raw tape (append-only crash recovery journal)
CREATE TABLE IF NOT EXISTS raw_tape (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    conversation_id INTEGER NOT NULL REFERENCES conversations(id),
    payload TEXT NOT NULL,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
);

-- Indexes for efficient queries
CREATE INDEX IF NOT EXISTS idx_messages_conversation ON messages(conversation_id);
CREATE INDEX IF NOT EXISTS idx_messages_seq ON messages(conversation_id, seq);
CREATE INDEX IF NOT EXISTS idx_summaries_conversation ON summaries(conversation_id);
CREATE INDEX IF NOT EXISTS idx_summaries_depth ON summaries(conversation_id, depth);
CREATE INDEX IF NOT EXISTS idx_context_items_conversation ON context_items(conversation_id);
CREATE INDEX IF NOT EXISTS idx_context_items_position ON context_items(conversation_id, position);

-- FTS5 full-text search on messages
CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
    content,
    content='messages',
    content_rowid='id'
);

-- Triggers to keep FTS in sync
CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
END;

CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.id, old.content);
END;

CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE ON messages BEGIN
    INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.id, old.content);
    INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
END;

-- FTS5 full-text search on summaries
CREATE VIRTUAL TABLE IF NOT EXISTS summaries_fts USING fts5(
    content,
    content='summaries',
    content_rowid='id'
);

CREATE TRIGGER IF NOT EXISTS summaries_ai AFTER INSERT ON summaries BEGIN
    INSERT INTO summaries_fts(rowid, content) VALUES (new.id, new.content);
END;

CREATE TRIGGER IF NOT EXISTS summaries_ad AFTER DELETE ON summaries BEGIN
    INSERT INTO summaries_fts(summaries_fts, rowid, content) VALUES('delete', old.id, old.content);
END;

CREATE TRIGGER IF NOT EXISTS summaries_au AFTER UPDATE ON summaries BEGIN
    INSERT INTO summaries_fts(summaries_fts, rowid, content) VALUES('delete', old.id, old.content);
    INSERT INTO summaries_fts(rowid, content) VALUES (new.id, new.content);
END;
"#;

/// SQL to check schema version and migrate if needed.
pub const VERSION_CHECK: &str = r#"
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER PRIMARY KEY
);
"#;

pub const CURRENT_VERSION: i32 = 1;
