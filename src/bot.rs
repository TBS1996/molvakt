use anyhow::Context;

use crate::db::{Conversation, Db, MessageRole, Participant, ParticipantRole};
use crate::flow::{self, LearnerSession};
use crate::llm::Llm;
use crate::whatsapp::WhatsApp;

#[derive(Clone)]
pub struct Bot {
    db: Db,
    whatsapp: WhatsApp,
}

impl Bot {
    pub async fn new(db: Db) -> anyhow::Result<Self> {
        Ok(Self {
            db,
            whatsapp: WhatsApp::from_env()?,
        })
    }

    pub async fn handle_webhook(&self, body: &str) -> anyhow::Result<()> {
        let messages = WhatsApp::parse_incoming_messages(body)?;
        if messages.is_empty() {
            println!("whatsapp webhook: no handled messages in payload");
            return Ok(());
        }
        for (phone, text) in messages {
            println!("whatsapp webhook: message from {phone}: {text:?}");
            if let Err(error) = self.handle_message(&phone, &text).await {
                eprintln!("error handling message from {phone}: {error:?}");
                let _ = self
                    .whatsapp
                    .send_text(
                        &phone,
                        "Sorry, something went wrong. Please try again in a moment.",
                    )
                    .await;
            }
        }
        Ok(())
    }

    async fn handle_message(&self, phone: &str, text: &str) -> anyhow::Result<()> {
        let conversation = self.db.get_or_create_default_conversation().await?;

        let participant = match self.db.find_participant_by_phone(phone).await? {
            Some(participant) => participant,
            None => return self.handle_registration(phone, text, conversation.id).await,
        };

        match participant.role {
            ParticipantRole::Teacher => {
                self.handle_teacher_message(&participant, text, &conversation)
                    .await
            }
            ParticipantRole::Learner => {
                self.handle_learner_message(&participant, text, &conversation)
                    .await
            }
        }
    }

    async fn handle_registration(
        &self,
        phone: &str,
        text: &str,
        conversation_id: i64,
    ) -> anyhow::Result<()> {
        match text.trim().to_ascii_uppercase().as_str() {
            "TEACHER" => {
                if self.db.find_participant_by_role(conversation_id, ParticipantRole::Teacher).await?.is_some() {
                    self.whatsapp
                        .send_text(phone, "A teacher is already registered for this conversation.")
                        .await?;
                    return Ok(());
                }
                self.db
                    .register_participant(conversation_id, phone, ParticipantRole::Teacher)
                    .await?;
                let conversation = self.db.get_conversation(conversation_id).await?;
                self.whatsapp
                    .send_text(
                        phone,
                        &format!(
                            "You're registered as the teacher. Send messages in {} and molvakt will forward them to the learner.",
                            conversation.target_language
                        ),
                    )
                    .await?;
            }
            "LEARNER" => {
                if self.db.find_participant_by_role(conversation_id, ParticipantRole::Learner).await?.is_some() {
                    self.whatsapp
                        .send_text(phone, "A learner is already registered for this conversation.")
                        .await?;
                    return Ok(());
                }
                let participant = self
                    .db
                    .register_participant(conversation_id, phone, ParticipantRole::Learner)
                    .await?;
                self.db.init_learner_session(participant.id).await?;
                let conversation = self.db.get_conversation(conversation_id).await?;
                self.whatsapp
                    .send_text(
                        phone,
                        &format!(
                            "You're registered as the learner. You'll practice {} here — reply to prompts as they come.",
                            conversation.target_language
                        ),
                    )
                    .await?;
            }
            _ => {
                self.whatsapp
                    .send_text(
                        phone,
                        "Welcome to molvakt!\n\n\
                         Reply TEACHER if you're the native speaker.\n\
                         Reply LEARNER if you're practicing the language.",
                    )
                    .await?;
            }
        }
        Ok(())
    }

    async fn handle_teacher_message(
        &self,
        teacher: &Participant,
        text: &str,
        conversation: &Conversation,
    ) -> anyhow::Result<()> {
        let learner = self
            .db
            .find_participant_by_role(conversation.id, ParticipantRole::Learner)
            .await?
            .context("no learner registered yet")?;

        let mut session = self.db.load_learner_session(learner.id).await?;
        if session != LearnerSession::Idle {
            self.whatsapp
                .send_text(
                    &teacher.phone,
                    "Please wait — the learner is still working on your last message.",
                )
                .await?;
            return Ok(());
        }

        let llm = Llm::from_env(conversation)?;
        let judgment = llm.validate_teacher_message(text).await?;
        if !judgment.accepted {
            let feedback = format!("Rejected: {}", judgment.feedback);
            self.whatsapp
                .send_text(&teacher.phone, &feedback)
                .await?;
            return Ok(());
        }

        self.db
            .insert_message(conversation.id, MessageRole::Teacher, text)
            .await?;

        let (new_session, teacher_message) = flow::begin_review(text.to_string());
        self.db
            .save_learner_session(learner.id, &new_session)
            .await?;

        self.whatsapp
            .send_text(&learner.phone, &teacher_message)
            .await?;
        self.send_review_choices(&learner.phone).await?;

        self.whatsapp
            .send_text(&teacher.phone, "Message sent to the learner.")
            .await?;

        Ok(())
    }

    async fn handle_learner_message(
        &self,
        learner: &Participant,
        text: &str,
        conversation: &Conversation,
    ) -> anyhow::Result<()> {
        let llm = Llm::from_env(conversation)?;
        let history = self.db.load_history(conversation.id).await?;
        let mut session = self.db.load_learner_session(learner.id).await?;

        let turn = flow::handle_learner_message(&mut session, text, &history, &llm).await?;
        self.db.save_learner_session(learner.id, &session).await?;

        for message in turn.learner_messages {
            self.whatsapp.send_text(&learner.phone, &message).await?;
        }

        if turn.show_review_choices {
            self.send_review_choices(&learner.phone).await?;
        }

        if let Some(learner_reply) = turn.completed_reply {
            self.db
                .insert_message(conversation.id, MessageRole::Learner, &learner_reply)
                .await?;

            if let Some(teacher) = self
                .db
                .find_participant_by_role(conversation.id, ParticipantRole::Teacher)
                .await?
            {
                if let Some(summary) = turn.teacher_message {
                    self.whatsapp.send_text(&teacher.phone, &summary).await?;
                }
            }
        }

        Ok(())
    }

    async fn send_review_choices(&self, phone: &str) -> anyhow::Result<()> {
        self.whatsapp
            .send_review_choice_list(
                phone,
                flow::review_choice_body(),
                flow::REVIEW_CHOICE_LIST_BUTTON,
                &flow::review_choice_list_rows(),
            )
            .await
    }
}
