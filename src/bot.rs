use anyhow::Context;

use crate::conversations::{
    handle_help, handle_list, handle_switch, is_help_command, is_list_command, is_new_conversation_command,
    parse_switch_selection,
};
use crate::db::{Conversation, Db, MessageRole, Participant, ParticipantResolve, ParticipantRole};
use crate::flow::{self, LearnerSession};
use crate::llm::Llm;
use crate::onboarding;
use crate::phone::display_phone;
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
        if let Some(invite) = self.db.find_pending_invite_for_phone(phone).await? {
            return onboarding::handle_invite_response(&self.db, &self.whatsapp, phone, text, invite)
                .await;
        }

        if self.db.load_onboarding_session(phone).await?.is_some() {
            return onboarding::handle_new_or_onboarding_user(
                &self.db,
                &self.whatsapp,
                phone,
                text,
            )
            .await;
        }

        if is_list_command(text) {
            return handle_list(&self.db, &self.whatsapp, phone).await;
        }

        if is_help_command(text) {
            return handle_help(&self.whatsapp, phone).await;
        }

        if let Some(selection) = parse_switch_selection(text) {
            return handle_switch(&self.db, &self.whatsapp, phone, selection).await;
        }

        if let Some(role) = is_new_conversation_command(text) {
            return onboarding::start_new_conversation(&self.db, &self.whatsapp, phone, role).await;
        }

        match self.db.resolve_participant_for_message(phone).await? {
            ParticipantResolve::NotRegistered => {
                onboarding::handle_new_or_onboarding_user(&self.db, &self.whatsapp, phone, text)
                    .await
            }
            ParticipantResolve::PickConversation => {
                self.whatsapp
                    .send_text(
                        phone,
                        "You have multiple conversations. Reply LIST to see them, then SWITCH <number> to pick one.",
                    )
                    .await?;
                handle_list(&self.db, &self.whatsapp, phone).await
            }
            ParticipantResolve::WaitingInvite { invite, .. } => {
                self.whatsapp
                    .send_text(
                        phone,
                        &format!(
                            "Still waiting for {} to accept your invite.",
                            display_phone(&invite.invitee_phone)
                        ),
                    )
                    .await?;
                Ok(())
            }
            ParticipantResolve::StaleIncomplete { conversation_id } => {
                self.db.delete_conversation(conversation_id).await?;
                self.whatsapp
                    .send_text(
                        phone,
                        "That conversation is no longer active. Reply LEARNER or TEACHER to start a new one.",
                    )
                    .await?;
                Ok(())
            }
            ParticipantResolve::Ready(participant) => {
                let conversation = self.db.get_conversation(participant.conversation_id).await?;
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
        }
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

        let session = self.db.load_learner_session(learner.id).await?;
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
        self.db
            .set_active_conversation(&learner.phone, conversation.id)
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
