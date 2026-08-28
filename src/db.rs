use anyhow::Context;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use sqlx::Row;
use std::path::PathBuf;
use std::str::FromStr;

use crate::flow::LearnerSession;
use crate::history::HistoryEntry;

#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

pub struct Conversation {
    pub id: i64,
    pub target_language: String,
    pub source_language: String,
}

pub struct Participant {
    pub id: i64,
    pub conversation_id: i64,
    pub phone: String,
    pub role: ParticipantRole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParticipantRole {
    Teacher,
    Learner,
}

#[derive(Clone, Copy)]
pub enum MessageRole {
    Teacher,
    Learner,
}

impl ParticipantRole {
    fn as_str(self) -> &'static str {
        match self {
            ParticipantRole::Teacher => "teacher",
            ParticipantRole::Learner => "learner",
        }
    }

    fn from_str(role: &str) -> anyhow::Result<Self> {
        match role {
            "teacher" => Ok(ParticipantRole::Teacher),
            "learner" => Ok(ParticipantRole::Learner),
            other => anyhow::bail!("unknown participant role: {other}"),
        }
    }
}

impl MessageRole {
    fn as_str(self) -> &'static str {
        match self {
            MessageRole::Teacher => "teacher",
            MessageRole::Learner => "learner",
        }
    }

    fn from_str(role: &str) -> anyhow::Result<Self> {
        match role {
            "teacher" => Ok(MessageRole::Teacher),
            "learner" => Ok(MessageRole::Learner),
            other => anyhow::bail!("unknown message role: {other}"),
        }
    }
}

impl Db {
    pub async fn connect() -> anyhow::Result<Self> {
        let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:languagebot.db".into());
        ensure_sqlite_parent_exists(&url)?;
        let options = SqliteConnectOptions::from_str(&url)
            .context("invalid DATABASE_URL")?
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options)
            .await
            .with_context(|| format!("failed to connect to database at {url}"))?;
        sqlx::migrate!()
            .run(&pool)
            .await
            .context("failed to run database migrations")?;
        Ok(Self { pool })
    }

    pub async fn get_or_create_default_conversation(&self) -> anyhow::Result<Conversation> {
        if let Some(conversation) = self.load_conversation_by_id_optional().await? {
            return Ok(conversation);
        }

        let target_language = std::env::var("MOLVAKT_TARGET_LANGUAGE")
            .unwrap_or_else(|_| "Norwegian".into());
        let source_language = std::env::var("MOLVAKT_SOURCE_LANGUAGE")
            .unwrap_or_else(|_| "English".into());

        let result = sqlx::query(
            "INSERT INTO conversations (target_language, source_language) VALUES (?, ?)",
        )
        .bind(&target_language)
        .bind(&source_language)
        .execute(&self.pool)
        .await?;

        Ok(Conversation {
            id: result.last_insert_rowid(),
            target_language,
            source_language,
        })
    }

    async fn load_conversation_by_id_optional(&self) -> anyhow::Result<Option<Conversation>> {
        let row = sqlx::query(
            "SELECT id, target_language, source_language FROM conversations ORDER BY id LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| Conversation {
            id: row.get("id"),
            target_language: row.get("target_language"),
            source_language: row.get("source_language"),
        }))
    }

    pub async fn load_history(&self, conversation_id: i64) -> anyhow::Result<Vec<HistoryEntry>> {
        let rows = sqlx::query(
            "SELECT role, content FROM messages WHERE conversation_id = ? ORDER BY id ASC",
        )
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                let role: String = row.get("role");
                let content: String = row.get("content");
                match MessageRole::from_str(&role)? {
                    MessageRole::Teacher => Ok(HistoryEntry::Teacher(content)),
                    MessageRole::Learner => Ok(HistoryEntry::Learner(content)),
                }
            })
            .collect()
    }

    pub async fn insert_message(
        &self,
        conversation_id: i64,
        role: MessageRole,
        content: &str,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO messages (conversation_id, role, content) VALUES (?, ?, ?)",
        )
        .bind(conversation_id)
        .bind(role.as_str())
        .bind(content)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_conversation(&self, conversation_id: i64) -> anyhow::Result<Conversation> {
        let row = sqlx::query(
            "SELECT id, target_language, source_language FROM conversations WHERE id = ?",
        )
        .bind(conversation_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(Conversation {
            id: row.get("id"),
            target_language: row.get("target_language"),
            source_language: row.get("source_language"),
        })
    }

    pub async fn find_participant_by_phone(
        &self,
        phone: &str,
    ) -> anyhow::Result<Option<Participant>> {
        let row = sqlx::query(
            "SELECT id, conversation_id, phone, role FROM participants WHERE phone = ?",
        )
        .bind(phone)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| Participant {
            id: row.get("id"),
            conversation_id: row.get("conversation_id"),
            phone: row.get("phone"),
            role: ParticipantRole::from_str(row.get::<String, _>("role").as_str()).unwrap(),
        }))
    }

    pub async fn find_participant_by_role(
        &self,
        conversation_id: i64,
        role: ParticipantRole,
    ) -> anyhow::Result<Option<Participant>> {
        let row = sqlx::query(
            "SELECT id, conversation_id, phone, role FROM participants WHERE conversation_id = ? AND role = ?",
        )
        .bind(conversation_id)
        .bind(role.as_str())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|row| Participant {
            id: row.get("id"),
            conversation_id: row.get("conversation_id"),
            phone: row.get("phone"),
            role: ParticipantRole::from_str(row.get::<String, _>("role").as_str()).unwrap(),
        }))
    }

    pub async fn register_participant(
        &self,
        conversation_id: i64,
        phone: &str,
        role: ParticipantRole,
    ) -> anyhow::Result<Participant> {
        let result = sqlx::query(
            "INSERT INTO participants (conversation_id, phone, role) VALUES (?, ?, ?)",
        )
        .bind(conversation_id)
        .bind(phone)
        .bind(role.as_str())
        .execute(&self.pool)
        .await?;

        Ok(Participant {
            id: result.last_insert_rowid(),
            conversation_id,
            phone: phone.to_string(),
            role,
        })
    }

    pub async fn init_learner_session(&self, participant_id: i64) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO learner_sessions (participant_id, session_json) VALUES (?, ?)",
        )
        .bind(participant_id)
        .bind(serde_json::to_string(&LearnerSession::Idle)?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn load_learner_session(
        &self,
        participant_id: i64,
    ) -> anyhow::Result<LearnerSession> {
        let row = sqlx::query(
            "SELECT session_json FROM learner_sessions WHERE participant_id = ?",
        )
        .bind(participant_id)
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(row) => {
                let json: String = row.get("session_json");
                Ok(serde_json::from_str(&json)?)
            }
            None => Ok(LearnerSession::Idle),
        }
    }

    pub async fn save_learner_session(
        &self,
        participant_id: i64,
        session: &LearnerSession,
    ) -> anyhow::Result<()> {
        let json = serde_json::to_string(session)?;
        sqlx::query(
            "INSERT INTO learner_sessions (participant_id, session_json, updated_at)
             VALUES (?, ?, datetime('now'))
             ON CONFLICT(participant_id) DO UPDATE SET
               session_json = excluded.session_json,
               updated_at = excluded.updated_at",
        )
        .bind(participant_id)
        .bind(json)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

fn ensure_sqlite_parent_exists(url: &str) -> anyhow::Result<()> {
    let Some(path) = sqlite_file_path(url) else {
        return Ok(());
    };
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create {}", parent.display()))
}

fn sqlite_file_path(url: &str) -> Option<PathBuf> {
    if let Some(path) = url.strip_prefix("sqlite:///") {
        return Some(path.into());
    }
    if let Some(path) = url.strip_prefix("sqlite:") {
        return Some(path.into());
    }
    None
}
