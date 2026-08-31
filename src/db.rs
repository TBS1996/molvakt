use anyhow::Context;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use sqlx::Row;
use std::path::PathBuf;
use std::str::FromStr;

use crate::flow::LearnerSession;
use crate::history::HistoryEntry;
use crate::phone::normalize_phone;

#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

pub struct Conversation {
    pub id: i64,
    pub mode: ConversationMode,
    pub target_language: String,
    pub source_language: String,
    pub exchange_turn_phone: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConversationMode {
    Tutor,
    Exchange,
    ExchangeTurns,
}

impl ConversationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tutor => "tutor",
            Self::Exchange => "exchange",
            Self::ExchangeTurns => "exchange_turns",
        }
    }

    pub fn from_str(mode: &str) -> anyhow::Result<Self> {
        match mode {
            "tutor" => Ok(Self::Tutor),
            "exchange" => Ok(Self::Exchange),
            "exchange_turns" => Ok(Self::ExchangeTurns),
            other => anyhow::bail!("unknown conversation mode: {other}"),
        }
    }

    pub fn is_exchange(self) -> bool {
        matches!(self, Self::Exchange | Self::ExchangeTurns)
    }
}

#[derive(Clone)]
pub struct Participant {
    pub id: i64,
    pub conversation_id: i64,
    pub phone: String,
    pub role: ParticipantRole,
    pub learning_language: Option<String>,
}

pub struct ConversationListing {
    pub conversation_id: i64,
    pub mode: ConversationMode,
    pub target_language: String,
    pub role: ParticipantRole,
    pub learning_language: Option<String>,
    pub partner_learning_language: Option<String>,
    pub partner_phone: Option<String>,
    pub partner_display_name: Option<String>,
    pub is_active: bool,
    pub is_pending: bool,
    pub turn: Option<ConversationTurnStatus>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationTurnStatus {
    YourTurnToSend,
    YourTurnToReply,
    WaitingForReply,
    WaitingForMessage,
}

impl ConversationTurnStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::YourTurnToSend => "your turn to send",
            Self::YourTurnToReply => "your turn to reply",
            Self::WaitingForReply => "waiting for reply",
            Self::WaitingForMessage => "waiting for message",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationModeSetting {
    Teacher,
    Learner,
    Exchange,
    ExchangeTurns,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetModeError {
    NotParticipant,
    NotComplete,
    ActiveExchange,
    MissingLanguage,
}

impl std::fmt::Display for SetModeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotParticipant => write!(f, "not a participant in this conversation"),
            Self::NotComplete => write!(f, "conversation is not ready for mode change"),
            Self::ActiveExchange => {
                write!(f, "learner is mid-exchange; wait until the current message is finished")
            }
            Self::MissingLanguage => write!(f, "missing learning language for mode change"),
        }
    }
}

impl std::error::Error for SetModeError {}

pub enum ParticipantResolve {
    Ready(Participant),
    WaitingInvite {
        participant: Participant,
        invite: ConversationInvite,
    },
    StaleIncomplete {
        conversation_id: i64,
    },
    PickConversation,
    NotRegistered,
}

pub struct ConversationInvite {
    pub id: i64,
    pub conversation_id: i64,
    pub inviter_phone: String,
    pub invitee_phone: String,
    pub inviter_role: ParticipantRole,
    pub status: InviteStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InviteStatus {
    Pending,
    Accepted,
    Declined,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParticipantRole {
    Teacher,
    Learner,
}

#[derive(Clone, Copy)]
pub enum MessageRole {
    Teacher,
    Learner,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OnboardingData {
    pub mode: Option<ConversationMode>,
    pub role: Option<ParticipantRole>,
    pub partner_phone: Option<String>,
    pub target_language: Option<String>,
    pub source_language: Option<String>,
    pub pending_conversation_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnboardingStep {
    Welcome,
    EnterPartnerPhone,
    EnterTargetLanguage,
    EnterExchangeLearningLanguage,
}

impl OnboardingStep {
    fn as_str(self) -> &'static str {
        match self {
            Self::Welcome => "welcome",
            Self::EnterPartnerPhone => "enter_partner_phone",
            Self::EnterTargetLanguage => "enter_target_language",
            Self::EnterExchangeLearningLanguage => "enter_exchange_learning_language",
        }
    }

    fn from_str(step: &str) -> anyhow::Result<Self> {
        if step.starts_with("menu_") {
            anyhow::bail!("not an onboarding step: {step}");
        }
        match step {
            "welcome" => Ok(Self::Welcome),
            "enter_partner_phone" => Ok(Self::EnterPartnerPhone),
            "enter_target_language" => Ok(Self::EnterTargetLanguage),
            "enter_exchange_learning_language" => Ok(Self::EnterExchangeLearningLanguage),
            other => anyhow::bail!("unknown onboarding step: {other}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MenuAction {
    Switch,
    SetMode,
    SetLanguage,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MenuData {
    pub action: Option<MenuAction>,
    pub conversation_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuStep {
    PickConversation,
    PickMode,
    AwaitLanguage,
}

impl MenuStep {
    fn as_str(self) -> &'static str {
        match self {
            Self::PickConversation => "menu_pick_conversation",
            Self::PickMode => "menu_pick_mode",
            Self::AwaitLanguage => "menu_await_language",
        }
    }

    fn from_str(step: &str) -> anyhow::Result<Self> {
        match step {
            "menu_pick_conversation" => Ok(Self::PickConversation),
            "menu_pick_mode" => Ok(Self::PickMode),
            "menu_await_language" => Ok(Self::AwaitLanguage),
            other => anyhow::bail!("unknown menu step: {other}"),
        }
    }
}

impl ParticipantRole {
    pub fn as_str(self) -> &'static str {
        match self {
            ParticipantRole::Teacher => "teacher",
            ParticipantRole::Learner => "learner",
        }
    }

    pub fn label(self) -> &'static str {
        self.as_str()
    }

    pub fn from_str(role: &str) -> anyhow::Result<Self> {
        match role {
            "teacher" => Ok(ParticipantRole::Teacher),
            "learner" => Ok(ParticipantRole::Learner),
            other => anyhow::bail!("unknown participant role: {other}"),
        }
    }

    pub fn opposite(self) -> Self {
        match self {
            ParticipantRole::Teacher => ParticipantRole::Learner,
            ParticipantRole::Learner => ParticipantRole::Teacher,
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

impl InviteStatus {
    fn as_str(self) -> &'static str {
        match self {
            InviteStatus::Pending => "pending",
            InviteStatus::Accepted => "accepted",
            InviteStatus::Declined => "declined",
        }
    }

    fn from_str(status: &str) -> anyhow::Result<Self> {
        match status {
            "pending" => Ok(InviteStatus::Pending),
            "accepted" => Ok(InviteStatus::Accepted),
            "declined" => Ok(InviteStatus::Declined),
            other => anyhow::bail!("unknown invite status: {other}"),
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

    pub async fn create_conversation(
        &self,
        mode: ConversationMode,
        target_language: &str,
        source_language: &str,
    ) -> anyhow::Result<Conversation> {
        let result = sqlx::query(
            "INSERT INTO conversations (mode, target_language, source_language) VALUES (?, ?, ?)",
        )
        .bind(mode.as_str())
        .bind(target_language)
        .bind(source_language)
        .execute(&self.pool)
        .await?;

        Ok(Conversation {
            id: result.last_insert_rowid(),
            mode,
            target_language: target_language.to_string(),
            source_language: source_language.to_string(),
            exchange_turn_phone: None,
        })
    }

    pub async fn load_history(&self, conversation_id: i64) -> anyhow::Result<Vec<HistoryEntry>> {
        let conversation = self.get_conversation(conversation_id).await?;
        let rows = sqlx::query(
            "SELECT role, content, sender_phone FROM messages WHERE conversation_id = ? ORDER BY id ASC",
        )
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await?;

        if conversation.mode.is_exchange() {
            let participants = self
                .find_participants_in_conversation(conversation_id)
                .await?;
            return rows
                .into_iter()
                .map(|row| {
                    let content: String = row.get("content");
                    let sender_phone: Option<String> = row.get("sender_phone");
                    let sender_phone = sender_phone.unwrap_or_default();
                    let sender_label = participants
                        .iter()
                        .find(|participant| participant.phone == sender_phone)
                        .map(|participant| participant.phone.clone())
                        .unwrap_or(sender_phone);
                    Ok(HistoryEntry::Exchange {
                        sender: sender_label,
                        content,
                    })
                })
                .collect();
        }

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
        self.insert_message_from(conversation_id, role, content, None)
            .await
    }

    pub async fn insert_exchange_message(
        &self,
        conversation_id: i64,
        sender: &Participant,
        content: &str,
    ) -> anyhow::Result<()> {
        let role = match sender.role {
            ParticipantRole::Teacher => MessageRole::Teacher,
            ParticipantRole::Learner => MessageRole::Learner,
        };
        self.insert_message_from(conversation_id, role, content, Some(&sender.phone))
            .await
    }

    async fn insert_message_from(
        &self,
        conversation_id: i64,
        role: MessageRole,
        content: &str,
        sender_phone: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO messages (conversation_id, role, content, sender_phone) VALUES (?, ?, ?, ?)",
        )
        .bind(conversation_id)
        .bind(role.as_str())
        .bind(content)
        .bind(sender_phone.map(normalize_phone))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_conversation(&self, conversation_id: i64) -> anyhow::Result<Conversation> {
        let row = sqlx::query(
            "SELECT id, mode, target_language, source_language, exchange_turn_phone
             FROM conversations WHERE id = ?",
        )
        .bind(conversation_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(Conversation {
            id: row.get("id"),
            mode: ConversationMode::from_str(row.get::<String, _>("mode").as_str())?,
            target_language: row.get("target_language"),
            source_language: row.get("source_language"),
            exchange_turn_phone: row.get("exchange_turn_phone"),
        })
    }

    pub async fn exchange_is_ready(&self, conversation_id: i64) -> anyhow::Result<bool> {
        let participants = self
            .find_participants_in_conversation(conversation_id)
            .await?;
        Ok(participants.len() >= 2
            && participants
                .iter()
                .all(|participant| participant.learning_language.is_some()))
    }

    pub async fn resolve_participant_for_message(
        &self,
        phone: &str,
    ) -> anyhow::Result<ParticipantResolve> {
        let phone = normalize_phone(phone);
        let participants = self.find_all_participants_by_phone(&phone).await?;

        if participants.is_empty() {
            return Ok(ParticipantResolve::NotRegistered);
        }

        let mut complete = Vec::new();
        let mut incomplete = Vec::new();
        for participant in participants {
            if self
                .conversation_has_both_participants(participant.conversation_id)
                .await?
            {
                complete.push(participant);
            } else {
                incomplete.push(participant);
            }
        }

        if let Some(active_id) = self.get_active_conversation_id(&phone).await? {
            if let Some(participant) = incomplete
                .iter()
                .find(|participant| participant.conversation_id == active_id)
            {
                if let Some(invite) = self
                    .find_pending_invite_for_conversation(participant.conversation_id)
                    .await?
                {
                    return Ok(ParticipantResolve::WaitingInvite {
                        participant: participant.clone(),
                        invite,
                    });
                }
                return Ok(ParticipantResolve::StaleIncomplete {
                    conversation_id: participant.conversation_id,
                });
            }

            if let Some(participant) = complete
                .iter()
                .find(|participant| participant.conversation_id == active_id)
            {
                return Ok(ParticipantResolve::Ready(participant.clone()));
            }
        }

        match complete.len() {
            0 => {
                if incomplete.len() == 1 {
                    let participant = incomplete.into_iter().next().unwrap();
                    if let Some(invite) = self
                        .find_pending_invite_for_conversation(participant.conversation_id)
                        .await?
                    {
                        return Ok(ParticipantResolve::WaitingInvite {
                            participant,
                            invite,
                        });
                    }
                    return Ok(ParticipantResolve::StaleIncomplete {
                        conversation_id: participant.conversation_id,
                    });
                }
                Ok(ParticipantResolve::NotRegistered)
            }
            1 => {
                let participant = complete.into_iter().next().unwrap();
                self.set_active_conversation(&phone, participant.conversation_id)
                    .await?;
                Ok(ParticipantResolve::Ready(participant))
            }
            _ => Ok(ParticipantResolve::PickConversation),
        }
    }

    pub async fn list_conversations_for_phone(
        &self,
        phone: &str,
    ) -> anyhow::Result<Vec<ConversationListing>> {
        let phone = normalize_phone(phone);
        let active_id = self.get_active_conversation_id(&phone).await?;
        let participants = self.find_all_participants_by_phone(&phone).await?;
        let mut listings = Vec::new();

        for participant in participants {
            let conversation = self.get_conversation(participant.conversation_id).await?;
            let is_pending = !self
                .conversation_has_both_participants(participant.conversation_id)
                .await?;
            let partner_phone = if is_pending {
                self.find_pending_invite_for_conversation(participant.conversation_id)
                    .await?
                    .map(|invite| {
                        if invite.inviter_phone == phone {
                            invite.invitee_phone
                        } else {
                            invite.inviter_phone
                        }
                    })
            } else {
                self.find_partner_phone(participant.conversation_id, &phone)
                    .await?
            };

            let partner_display_name = if let Some(ref partner_phone) = partner_phone {
                self.get_display_name(partner_phone).await?
            } else {
                None
            };

            let partner_learning_language = if is_pending {
                None
            } else if let Some(ref partner_phone) = partner_phone {
                self.find_participant_in_conversation(partner_phone, participant.conversation_id)
                    .await?
                    .and_then(|partner| partner.learning_language)
            } else {
                None
            };

            let turn = if is_pending {
                None
            } else if conversation.mode == ConversationMode::ExchangeTurns {
                if self.exchange_is_ready(participant.conversation_id).await? {
                    Some(
                        self.turn_status_for_exchange_turns(&conversation, &participant)
                            .await?,
                    )
                } else {
                    Some(ConversationTurnStatus::WaitingForMessage)
                }
            } else if conversation.mode == ConversationMode::Exchange {
                if self.exchange_is_ready(participant.conversation_id).await? {
                    Some(ConversationTurnStatus::YourTurnToSend)
                } else {
                    Some(ConversationTurnStatus::WaitingForMessage)
                }
            } else {
                Some(
                    self.turn_status_for_participant(&participant)
                        .await?,
                )
            };

            listings.push(ConversationListing {
                conversation_id: participant.conversation_id,
                mode: conversation.mode,
                target_language: conversation.target_language,
                role: participant.role,
                learning_language: participant.learning_language.clone(),
                partner_learning_language,
                partner_phone,
                partner_display_name,
                is_active: active_id == Some(participant.conversation_id),
                is_pending,
                turn,
            });
        }

        listings.sort_by(|left, right| {
            right
                .is_active
                .cmp(&left.is_active)
                .then_with(|| left.is_pending.cmp(&right.is_pending))
                .then_with(|| left.conversation_id.cmp(&right.conversation_id))
        });

        Ok(listings)
    }

    pub async fn turn_status_for_exchange_turns(
        &self,
        conversation: &Conversation,
        participant: &Participant,
    ) -> anyhow::Result<ConversationTurnStatus> {
        let Some(turn_phone) = conversation.exchange_turn_phone.as_deref() else {
            return Ok(ConversationTurnStatus::YourTurnToSend);
        };

        if participant.phone == turn_phone {
            Ok(ConversationTurnStatus::YourTurnToSend)
        } else {
            Ok(ConversationTurnStatus::WaitingForMessage)
        }
    }

    pub async fn init_exchange_turn_state(
        &self,
        conversation_id: i64,
        first_turn_phone: &str,
    ) -> anyhow::Result<()> {
        let phone = normalize_phone(first_turn_phone);
        sqlx::query(
            "UPDATE conversations
             SET exchange_turn_phone = ?, exchange_awaiting_reply = 0
             WHERE id = ?",
        )
        .bind(&phone)
        .bind(conversation_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn clear_exchange_turn_state(&self, conversation_id: i64) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE conversations
             SET exchange_turn_phone = NULL, exchange_awaiting_reply = 0
             WHERE id = ?",
        )
        .bind(conversation_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn pass_exchange_turn(
        &self,
        conversation_id: i64,
        next_turn_phone: &str,
    ) -> anyhow::Result<()> {
        let phone = normalize_phone(next_turn_phone);
        sqlx::query("UPDATE conversations SET exchange_turn_phone = ? WHERE id = ?")
            .bind(&phone)
            .bind(conversation_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn assign_default_exchange_learning_languages(
        &self,
        conversation_id: i64,
        conversation: &Conversation,
        participants: &[Participant],
    ) -> anyhow::Result<()> {
        for participant in participants {
            if participant.role == ParticipantRole::Learner && participant.learning_language.is_none()
            {
                self.update_learning_language(
                    conversation_id,
                    &participant.phone,
                    &conversation.target_language,
                )
                .await?;
            }
            if participant.role == ParticipantRole::Teacher && participant.learning_language.is_none()
            {
                self.update_learning_language(
                    conversation_id,
                    &participant.phone,
                    &conversation.source_language,
                )
                .await?;
            }
        }
        Ok(())
    }

    pub async fn turn_status_for_participant(
        &self,
        participant: &Participant,
    ) -> anyhow::Result<ConversationTurnStatus> {
        match participant.role {
            ParticipantRole::Teacher => {
                let learner = self
                    .find_participant_by_role(participant.conversation_id, ParticipantRole::Learner)
                    .await?;
                let session = if let Some(learner) = learner {
                    self.load_learner_session(learner.id).await?
                } else {
                    LearnerSession::Idle
                };
                if session == LearnerSession::Idle {
                    Ok(ConversationTurnStatus::YourTurnToSend)
                } else {
                    Ok(ConversationTurnStatus::WaitingForReply)
                }
            }
            ParticipantRole::Learner => {
                let session = self.load_learner_session(participant.id).await?;
                if session == LearnerSession::Idle {
                    Ok(ConversationTurnStatus::WaitingForMessage)
                } else {
                    Ok(ConversationTurnStatus::YourTurnToReply)
                }
            }
        }
    }

    pub async fn apply_conversation_mode(
        &self,
        conversation_id: i64,
        phone: &str,
        setting: ConversationModeSetting,
    ) -> anyhow::Result<(Participant, Participant)> {
        let phone = normalize_phone(phone);
        if !self.conversation_has_both_participants(conversation_id).await? {
            return Err(SetModeError::NotComplete.into());
        }

        let conversation = self.get_conversation(conversation_id).await?;
        let participants = self.find_participants_in_conversation(conversation_id).await?;
        if participants
            .iter()
            .all(|participant| participant.phone != phone)
        {
            return Err(SetModeError::NotParticipant.into());
        }

        if conversation.mode == ConversationMode::Tutor {
            if let Some(learner) = participants
                .iter()
                .find(|participant| participant.role == ParticipantRole::Learner)
            {
                let session = self.load_learner_session(learner.id).await?;
                if session != LearnerSession::Idle {
                    return Err(SetModeError::ActiveExchange.into());
                }
            }
        }

        match setting {
            ConversationModeSetting::Exchange => {
                if conversation.mode == ConversationMode::Exchange {
                    let requester = self
                        .find_participant_in_conversation(&phone, conversation_id)
                        .await?
                        .context("requester missing in exchange conversation")
                        .map_err(|_| SetModeError::NotComplete)?;
                    let partner = self
                        .find_partner_participant(conversation_id, &phone)
                        .await?
                        .context("partner missing in exchange conversation")
                        .map_err(|_| SetModeError::NotComplete)?;
                    return Ok((requester, partner));
                }

                self.assign_default_exchange_learning_languages(
                    conversation_id,
                    &conversation,
                    &participants,
                )
                .await?;

                sqlx::query(
                    "UPDATE conversations
                     SET mode = 'exchange', exchange_turn_phone = NULL, exchange_awaiting_reply = 0
                     WHERE id = ?",
                )
                .bind(conversation_id)
                .execute(&self.pool)
                .await?;
            }
            ConversationModeSetting::ExchangeTurns => {
                if conversation.mode == ConversationMode::ExchangeTurns {
                    let requester = self
                        .find_participant_in_conversation(&phone, conversation_id)
                        .await?
                        .context("requester missing in exchange turns conversation")
                        .map_err(|_| SetModeError::NotComplete)?;
                    let partner = self
                        .find_partner_participant(conversation_id, &phone)
                        .await?
                        .context("partner missing in exchange turns conversation")
                        .map_err(|_| SetModeError::NotComplete)?;
                    return Ok((requester, partner));
                }

                self.assign_default_exchange_learning_languages(
                    conversation_id,
                    &conversation,
                    &participants,
                )
                .await?;

                sqlx::query("UPDATE conversations SET mode = 'exchange_turns' WHERE id = ?")
                    .bind(conversation_id)
                    .execute(&self.pool)
                    .await?;
                self.init_exchange_turn_state(conversation_id, &phone).await?;
            }
            ConversationModeSetting::Teacher | ConversationModeSetting::Learner => {
                let desired_role = match setting {
                    ConversationModeSetting::Teacher => ParticipantRole::Teacher,
                    ConversationModeSetting::Learner => ParticipantRole::Learner,
                    ConversationModeSetting::Exchange | ConversationModeSetting::ExchangeTurns => {
                        unreachable!()
                    }
                };

                if conversation.mode.is_exchange() {
                    let requester = participants
                        .iter()
                        .find(|participant| participant.phone == phone)
                        .context("requester missing")
                        .map_err(|_| SetModeError::NotComplete)?;
                    let partner = participants
                        .iter()
                        .find(|participant| participant.phone != phone)
                        .context("partner missing")
                        .map_err(|_| SetModeError::NotComplete)?;

                    let target_language = match desired_role {
                        ParticipantRole::Learner => requester
                            .learning_language
                            .clone()
                            .or_else(|| partner.learning_language.clone())
                            .unwrap_or_else(|| conversation.target_language.clone()),
                        ParticipantRole::Teacher => partner
                            .learning_language
                            .clone()
                            .or_else(|| requester.learning_language.clone())
                            .unwrap_or_else(|| conversation.target_language.clone()),
                    };

                    sqlx::query(
                        "UPDATE conversations
                         SET mode = 'tutor', target_language = ?, exchange_turn_phone = NULL, exchange_awaiting_reply = 0
                         WHERE id = ?",
                    )
                    .bind(&target_language)
                    .bind(conversation_id)
                    .execute(&self.pool)
                    .await?;

                    sqlx::query(
                        "UPDATE participants SET role = ? WHERE conversation_id = ? AND phone = ?",
                    )
                    .bind(desired_role.as_str())
                    .bind(conversation_id)
                    .bind(&phone)
                    .execute(&self.pool)
                    .await?;
                    sqlx::query(
                        "UPDATE participants SET role = ? WHERE conversation_id = ? AND phone = ?",
                    )
                    .bind(desired_role.opposite().as_str())
                    .bind(conversation_id)
                    .bind(&partner.phone)
                    .execute(&self.pool)
                    .await?;
                } else {
                    let requester = participants
                        .iter()
                        .find(|participant| participant.phone == phone)
                        .context("requester missing")
                        .map_err(|_| SetModeError::NotComplete)?;
                    if requester.role != desired_role {
                        sqlx::query(
                            "UPDATE participants SET role = CASE
                                WHEN role = 'teacher' THEN 'learner'
                                WHEN role = 'learner' THEN 'teacher'
                             END
                             WHERE conversation_id = ?",
                        )
                        .bind(conversation_id)
                        .execute(&self.pool)
                        .await?;
                    }
                }
            }
        }

        let requester = self
            .find_participant_in_conversation(&phone, conversation_id)
            .await?
            .context("requester missing after mode change")
            .map_err(|_| SetModeError::NotComplete)?;
        let partner = self
            .find_partner_participant(conversation_id, &phone)
            .await?
            .context("partner missing after mode change")
            .map_err(|_| SetModeError::NotComplete)?;

        let final_conversation = self.get_conversation(conversation_id).await?;
        if final_conversation.mode == ConversationMode::Tutor {
            let learner = if requester.role == ParticipantRole::Learner {
                &requester
            } else {
                &partner
            };
            self.save_learner_session(learner.id, &LearnerSession::Idle)
                .await?;
        }

        Ok((requester, partner))
    }

    pub async fn update_target_language(
        &self,
        conversation_id: i64,
        phone: &str,
        target_language: &str,
    ) -> anyhow::Result<()> {
        let phone = normalize_phone(phone);
        if self
            .find_participant_in_conversation(&phone, conversation_id)
            .await?
            .is_none()
        {
            anyhow::bail!("not a participant in this conversation");
        }

        sqlx::query("UPDATE conversations SET target_language = ? WHERE id = ?")
            .bind(target_language)
            .bind(conversation_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_learning_language(
        &self,
        conversation_id: i64,
        phone: &str,
        learning_language: &str,
    ) -> anyhow::Result<()> {
        let phone = normalize_phone(phone);
        sqlx::query(
            "UPDATE participants SET learning_language = ?
             WHERE conversation_id = ? AND phone = ?",
        )
        .bind(learning_language)
        .bind(conversation_id)
        .bind(phone)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn find_partner_participant(
        &self,
        conversation_id: i64,
        phone: &str,
    ) -> anyhow::Result<Option<Participant>> {
        let phone = normalize_phone(phone);
        let row = sqlx::query(
            "SELECT id, conversation_id, phone, role, learning_language
             FROM participants
             WHERE conversation_id = ? AND phone != ?
             LIMIT 1",
        )
        .bind(conversation_id)
        .bind(phone)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(participant_from_row))
    }

    pub async fn find_pending_invite_between(
        &self,
        phone_a: &str,
        phone_b: &str,
    ) -> anyhow::Result<Option<ConversationInvite>> {
        let phone_a = normalize_phone(phone_a);
        let phone_b = normalize_phone(phone_b);
        let row = sqlx::query(
            "SELECT id, conversation_id, inviter_phone, invitee_phone, inviter_role, status
             FROM conversation_invites
             WHERE status = 'pending'
               AND ((inviter_phone = ? AND invitee_phone = ?)
                 OR (inviter_phone = ? AND invitee_phone = ?))
             ORDER BY id DESC
             LIMIT 1",
        )
        .bind(&phone_a)
        .bind(&phone_b)
        .bind(&phone_b)
        .bind(&phone_a)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(invite_from_row))
    }

    pub async fn find_complete_conversation_between(
        &self,
        phone_a: &str,
        phone_b: &str,
    ) -> anyhow::Result<Option<i64>> {
        let phone_a = normalize_phone(phone_a);
        let phone_b = normalize_phone(phone_b);

        let conversation_id: Option<i64> = sqlx::query_scalar(
            "SELECT p1.conversation_id
             FROM participants p1
             JOIN participants p2 ON p1.conversation_id = p2.conversation_id
             WHERE p1.phone = ? AND p2.phone = ?",
        )
        .bind(phone_a)
        .bind(phone_b)
        .fetch_optional(&self.pool)
        .await?;

        let Some(conversation_id) = conversation_id else {
            return Ok(None);
        };

        if self.conversation_has_both_participants(conversation_id).await? {
            Ok(Some(conversation_id))
        } else {
            Ok(None)
        }
    }

    pub async fn find_partner_phone(
        &self,
        conversation_id: i64,
        phone: &str,
    ) -> anyhow::Result<Option<String>> {
        let phone = normalize_phone(phone);
        let partner_phone: Option<String> = sqlx::query_scalar(
            "SELECT phone FROM participants WHERE conversation_id = ? AND phone != ? LIMIT 1",
        )
        .bind(conversation_id)
        .bind(phone)
        .fetch_optional(&self.pool)
        .await?;

        Ok(partner_phone)
    }

    pub async fn find_participant_for_phone(
        &self,
        phone: &str,
    ) -> anyhow::Result<Option<Participant>> {
        match self.resolve_participant_for_message(phone).await? {
            ParticipantResolve::Ready(participant) => Ok(Some(participant)),
            _ => Ok(None),
        }
    }

    pub async fn find_participant_by_role(
        &self,
        conversation_id: i64,
        role: ParticipantRole,
    ) -> anyhow::Result<Option<Participant>> {
        let row = sqlx::query(
            "SELECT id, conversation_id, phone, role, learning_language
             FROM participants WHERE conversation_id = ? AND role = ?",
        )
        .bind(conversation_id)
        .bind(role.as_str())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(participant_from_row))
    }

    pub async fn conversation_has_both_participants(
        &self,
        conversation_id: i64,
    ) -> anyhow::Result<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM participants WHERE conversation_id = ?",
        )
        .bind(conversation_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(count >= 2)
    }

    pub async fn register_participant(
        &self,
        conversation_id: i64,
        phone: &str,
        role: ParticipantRole,
        learning_language: Option<&str>,
    ) -> anyhow::Result<Participant> {
        let phone = normalize_phone(phone);
        let result = sqlx::query(
            "INSERT INTO participants (conversation_id, phone, role, learning_language)
             VALUES (?, ?, ?, ?)",
        )
        .bind(conversation_id)
        .bind(&phone)
        .bind(role.as_str())
        .bind(learning_language)
        .execute(&self.pool)
        .await?;

        self.set_active_conversation(&phone, conversation_id).await?;

        Ok(Participant {
            id: result.last_insert_rowid(),
            conversation_id,
            phone,
            role,
            learning_language: learning_language.map(ToString::to_string),
        })
    }

    pub async fn set_active_conversation(
        &self,
        phone: &str,
        conversation_id: i64,
    ) -> anyhow::Result<()> {
        let phone = normalize_phone(phone);
        sqlx::query(
            "INSERT INTO user_settings (phone, active_conversation_id, updated_at)
             VALUES (?, ?, datetime('now'))
             ON CONFLICT(phone) DO UPDATE SET
               active_conversation_id = excluded.active_conversation_id,
               updated_at = excluded.updated_at",
        )
        .bind(&phone)
        .bind(conversation_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn upsert_display_name(&self, phone: &str, display_name: &str) -> anyhow::Result<()> {
        let phone = normalize_phone(phone);
        let display_name = display_name.trim();
        if display_name.is_empty() {
            return Ok(());
        }

        sqlx::query(
            "INSERT INTO user_settings (phone, display_name, updated_at)
             VALUES (?, ?, datetime('now'))
             ON CONFLICT(phone) DO UPDATE SET
               display_name = excluded.display_name,
               updated_at = excluded.updated_at",
        )
        .bind(&phone)
        .bind(display_name)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_display_name(&self, phone: &str) -> anyhow::Result<Option<String>> {
        let phone = normalize_phone(phone);
        let name: Option<String> = sqlx::query_scalar(
            "SELECT display_name FROM user_settings WHERE phone = ?",
        )
        .bind(phone)
        .fetch_optional(&self.pool)
        .await?;

        Ok(name.filter(|name| !name.trim().is_empty()))
    }

    pub async fn create_invite(
        &self,
        conversation_id: i64,
        inviter_phone: &str,
        invitee_phone: &str,
        inviter_role: ParticipantRole,
    ) -> anyhow::Result<ConversationInvite> {
        let inviter_phone = normalize_phone(inviter_phone);
        let invitee_phone = normalize_phone(invitee_phone);
        let result = sqlx::query(
            "INSERT INTO conversation_invites
             (conversation_id, inviter_phone, invitee_phone, inviter_role, status)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(conversation_id)
        .bind(&inviter_phone)
        .bind(&invitee_phone)
        .bind(inviter_role.as_str())
        .bind(InviteStatus::Pending.as_str())
        .execute(&self.pool)
        .await?;

        Ok(ConversationInvite {
            id: result.last_insert_rowid(),
            conversation_id,
            inviter_phone,
            invitee_phone,
            inviter_role,
            status: InviteStatus::Pending,
        })
    }

    pub async fn find_pending_invite_for_phone(
        &self,
        phone: &str,
    ) -> anyhow::Result<Option<ConversationInvite>> {
        let phone = normalize_phone(phone);
        let row = sqlx::query(
            "SELECT id, conversation_id, inviter_phone, invitee_phone, inviter_role, status
             FROM conversation_invites
             WHERE invitee_phone = ? AND status = 'pending'
             ORDER BY id DESC
             LIMIT 1",
        )
        .bind(phone)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(invite_from_row))
    }

    pub async fn find_pending_invite_for_conversation(
        &self,
        conversation_id: i64,
    ) -> anyhow::Result<Option<ConversationInvite>> {
        let row = sqlx::query(
            "SELECT id, conversation_id, inviter_phone, invitee_phone, inviter_role, status
             FROM conversation_invites
             WHERE conversation_id = ? AND status = 'pending'
             ORDER BY id DESC
             LIMIT 1",
        )
        .bind(conversation_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(invite_from_row))
    }

    pub async fn update_invite_status(
        &self,
        invite_id: i64,
        status: InviteStatus,
    ) -> anyhow::Result<()> {
        sqlx::query("UPDATE conversation_invites SET status = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(invite_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_invite(&self, invite_id: i64) -> anyhow::Result<ConversationInvite> {
        let row = sqlx::query(
            "SELECT id, conversation_id, inviter_phone, invitee_phone, inviter_role, status
             FROM conversation_invites WHERE id = ?",
        )
        .bind(invite_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(invite_from_row(row))
    }

    pub async fn save_onboarding_session(
        &self,
        phone: &str,
        step: OnboardingStep,
        data: &OnboardingData,
    ) -> anyhow::Result<()> {
        let phone = normalize_phone(phone);
        let data_json = serde_json::to_string(data)?;
        sqlx::query(
            "INSERT INTO onboarding_sessions (phone, step, data_json, updated_at)
             VALUES (?, ?, ?, datetime('now'))
             ON CONFLICT(phone) DO UPDATE SET
               step = excluded.step,
               data_json = excluded.data_json,
               updated_at = excluded.updated_at",
        )
        .bind(phone)
        .bind(step.as_str())
        .bind(data_json)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn load_onboarding_session(
        &self,
        phone: &str,
    ) -> anyhow::Result<Option<(OnboardingStep, OnboardingData)>> {
        let phone = normalize_phone(phone);
        let row = sqlx::query(
            "SELECT step, data_json FROM onboarding_sessions WHERE phone = ?",
        )
        .bind(phone)
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let step_str: String = row.get("step");
        if step_str.starts_with("menu_") {
            return Ok(None);
        }

        let step = OnboardingStep::from_str(&step_str)?;
        let data: OnboardingData = serde_json::from_str(row.get("data_json"))?;
        Ok(Some((step, data)))
    }

    pub async fn delete_conversation(&self, conversation_id: i64) -> anyhow::Result<()> {
        let participant_ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM participants WHERE conversation_id = ?",
        )
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await?;

        for participant_id in participant_ids {
            sqlx::query("DELETE FROM learner_sessions WHERE participant_id = ?")
                .bind(participant_id)
                .execute(&self.pool)
                .await?;
        }

        sqlx::query("DELETE FROM participants WHERE conversation_id = ?")
            .bind(conversation_id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM conversation_invites WHERE conversation_id = ?")
            .bind(conversation_id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM messages WHERE conversation_id = ?")
            .bind(conversation_id)
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "UPDATE user_settings SET active_conversation_id = NULL
             WHERE active_conversation_id = ?",
        )
        .bind(conversation_id)
        .execute(&self.pool)
        .await?;
        sqlx::query("DELETE FROM conversations WHERE id = ?")
            .bind(conversation_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn clear_onboarding_session(&self, phone: &str) -> anyhow::Result<()> {
        let phone = normalize_phone(phone);
        sqlx::query("DELETE FROM onboarding_sessions WHERE phone = ?")
            .bind(phone)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn save_menu_session(
        &self,
        phone: &str,
        step: MenuStep,
        data: &MenuData,
    ) -> anyhow::Result<()> {
        let phone = normalize_phone(phone);
        let data_json = serde_json::to_string(data)?;
        sqlx::query(
            "INSERT INTO onboarding_sessions (phone, step, data_json, updated_at)
             VALUES (?, ?, ?, datetime('now'))
             ON CONFLICT(phone) DO UPDATE SET
               step = excluded.step,
               data_json = excluded.data_json,
               updated_at = excluded.updated_at",
        )
        .bind(phone)
        .bind(step.as_str())
        .bind(data_json)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn load_menu_session(
        &self,
        phone: &str,
    ) -> anyhow::Result<Option<(MenuStep, MenuData)>> {
        let phone = normalize_phone(phone);
        let row = sqlx::query(
            "SELECT step, data_json FROM onboarding_sessions WHERE phone = ?",
        )
        .bind(phone)
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let step_str: String = row.get("step");
        if !step_str.starts_with("menu_") {
            return Ok(None);
        }

        let step = MenuStep::from_str(&step_str)?;
        let data: MenuData = serde_json::from_str(row.get("data_json"))?;
        Ok(Some((step, data)))
    }

    pub async fn clear_menu_session(&self, phone: &str) -> anyhow::Result<()> {
        self.clear_onboarding_session(phone).await
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

    pub async fn get_active_conversation_id(&self, phone: &str) -> anyhow::Result<Option<i64>> {
        let row = sqlx::query(
            "SELECT active_conversation_id FROM user_settings WHERE phone = ?",
        )
        .bind(phone)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(|row| row.get::<Option<i64>, _>("active_conversation_id")))
    }

    async fn find_participants_in_conversation(
        &self,
        conversation_id: i64,
    ) -> anyhow::Result<Vec<Participant>> {
        let rows = sqlx::query(
            "SELECT id, conversation_id, phone, role, learning_language
             FROM participants WHERE conversation_id = ?",
        )
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(participant_from_row).collect())
    }

    async fn find_participant_in_conversation(
        &self,
        phone: &str,
        conversation_id: i64,
    ) -> anyhow::Result<Option<Participant>> {
        let row = sqlx::query(
            "SELECT id, conversation_id, phone, role, learning_language
             FROM participants WHERE phone = ? AND conversation_id = ?",
        )
        .bind(phone)
        .bind(conversation_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(participant_from_row))
    }

    async fn find_all_participants_by_phone(
        &self,
        phone: &str,
    ) -> anyhow::Result<Vec<Participant>> {
        let rows = sqlx::query(
            "SELECT id, conversation_id, phone, role, learning_language
             FROM participants WHERE phone = ?",
        )
        .bind(phone)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(participant_from_row).collect())
    }
}

fn participant_from_row(row: sqlx::sqlite::SqliteRow) -> Participant {
    Participant {
        id: row.get("id"),
        conversation_id: row.get("conversation_id"),
        phone: row.get("phone"),
        role: ParticipantRole::from_str(row.get::<String, _>("role").as_str()).unwrap(),
        learning_language: row.get("learning_language"),
    }
}

fn invite_from_row(row: sqlx::sqlite::SqliteRow) -> ConversationInvite {
    ConversationInvite {
        id: row.get("id"),
        conversation_id: row.get("conversation_id"),
        inviter_phone: row.get("inviter_phone"),
        invitee_phone: row.get("invitee_phone"),
        inviter_role: ParticipantRole::from_str(row.get::<String, _>("inviter_role").as_str())
            .unwrap(),
        status: InviteStatus::from_str(row.get::<String, _>("status").as_str()).unwrap(),
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
