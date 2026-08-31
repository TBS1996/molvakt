use anyhow::Context;

use crate::conversations::{
    format_chat_label, handle_cancel, handle_help, handle_list, handle_set_language, handle_set_mode,
    handle_switch, is_help_command, is_list_command, is_new_conversation_command,
    parse_cancel_selection, parse_set_language, parse_set_mode, parse_switch_selection,
};
use crate::db::{
    Conversation, ConversationMode, Db, MessageRole, Participant, ParticipantResolve,
    ParticipantRole,
};
use crate::flow::{self, LearnerSession};
use crate::llm::Llm;
use crate::onboarding;
use crate::phone::{contact_label, partner_label};
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
        for message in messages {
            println!(
                "whatsapp webhook: message from {}: {:?}",
                message.from, message.text
            );
            if let Some(ref name) = message.profile_name {
                if let Err(error) = self.db.upsert_display_name(&message.from, name).await {
                    eprintln!(
                        "error saving display name for {}: {error:?}",
                        message.from
                    );
                }
            }
            if let Err(error) = self
                .handle_message(&message.from, &message.text)
                .await
            {
                eprintln!("error handling message from {}: {error:?}", message.from);
                let _ = self
                    .whatsapp
                    .send_text(
                        &message.from,
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

        if let Some(command) = parse_set_mode(text) {
            return handle_set_mode(&self.db, &self.whatsapp, phone, command).await;
        }

        if let Some(command) = parse_set_language(text) {
            return handle_set_language(&self.db, &self.whatsapp, phone, command).await;
        }

        if let Some(selection) = parse_cancel_selection(text) {
            return handle_cancel(&self.db, &self.whatsapp, phone, selection).await;
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
                let invitee_name = self.db.get_display_name(&invite.invitee_phone).await?;
                self.whatsapp
                    .send_text(
                        phone,
                        &format!(
                            "Still waiting for {} to accept your invite.",
                            contact_label(&invite.invitee_phone, invitee_name.as_deref())
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
                if conversation.mode == ConversationMode::Exchange {
                    return self
                        .handle_parallel_exchange_message(&participant, text, &conversation)
                        .await;
                }
                if conversation.mode == ConversationMode::ExchangeTurns {
                    return self
                        .handle_exchange_turns_message(&participant, text, &conversation)
                        .await;
                }
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

        let previous_active = self
            .db
            .get_active_conversation_id(&learner.phone)
            .await?;
        let teacher_name = self.db.get_display_name(&teacher.phone).await?;
        let (new_session, mut teacher_message) = flow::begin_review(
            text.to_string(),
            &partner_label(
                &teacher.phone,
                &conversation.target_language,
                teacher_name.as_deref(),
            ),
        );
        if previous_active.is_some() && previous_active != Some(conversation.id) {
            let chat_label = format_chat_label(
                ParticipantRole::Learner,
                &conversation.target_language,
                &teacher.phone,
                teacher_name.as_deref(),
            );
            teacher_message.push_str(&format!("\n\n(Switched active chat to {chat_label}.)"));
        }
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

        let learner_name = self.db.get_display_name(&learner.phone).await?;
        self.whatsapp
            .send_text(
                &teacher.phone,
                &format!(
                    "Message sent to {} (learner).",
                    contact_label(&learner.phone, learner_name.as_deref())
                ),
            )
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

        let learner_name = self.db.get_display_name(&learner.phone).await?;
        let learner_label = partner_label(
            &learner.phone,
            &conversation.target_language,
            learner_name.as_deref(),
        );
        let turn =
            flow::handle_learner_message(&mut session, text, &history, &llm, &learner_label)
                .await?;
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

    async fn handle_parallel_exchange_message(
        &self,
        sender: &Participant,
        text: &str,
        conversation: &Conversation,
    ) -> anyhow::Result<()> {
        let partner = self
            .db
            .find_partner_participant(conversation.id, &sender.phone)
            .await?
            .context("no exchange partner registered yet")?;
        let learning_language = sender
            .learning_language
            .as_deref()
            .unwrap_or(&conversation.target_language);
        let partner_learning_language = partner
            .learning_language
            .as_deref()
            .unwrap_or(&conversation.source_language);

        let history = self.db.load_history(conversation.id).await?;
        let llm = Llm::for_exchange(learning_language, partner_learning_language)?;
        let judgment = llm.validate_reply(text, &history).await?;
        if !judgment.accepted {
            self.whatsapp
                .send_text(&sender.phone, &judgment.feedback)
                .await?;
            return Ok(());
        }

        self.db
            .insert_exchange_message(conversation.id, sender, text)
            .await?;

        let sender_name = self.db.get_display_name(&sender.phone).await?;
        let sender_label = contact_label(&sender.phone, sender_name.as_deref());
        let mut outgoing = format!("[{sender_label}]: {text}");

        let previous_active = self
            .db
            .get_active_conversation_id(&partner.phone)
            .await?;
        if previous_active.is_some() && previous_active != Some(conversation.id) {
            let chat_label = format!(
                "exchange — you learn {partner_learning_language} with {sender_label}"
            );
            outgoing.push_str(&format!("\n\n(Switched active chat to {chat_label}.)"));
        }
        self.db
            .set_active_conversation(&partner.phone, conversation.id)
            .await?;

        self.whatsapp.send_text(&partner.phone, &outgoing).await?;

        let partner_name = self.db.get_display_name(&partner.phone).await?;
        self.whatsapp
            .send_text(
                &sender.phone,
                &format!(
                    "Message sent to {}.",
                    contact_label(&partner.phone, partner_name.as_deref())
                ),
            )
            .await?;

        Ok(())
    }

    async fn handle_exchange_turns_message(
        &self,
        sender: &Participant,
        text: &str,
        conversation: &Conversation,
    ) -> anyhow::Result<()> {
        let partner = self
            .db
            .find_partner_participant(conversation.id, &sender.phone)
            .await?
            .context("no exchange partner registered yet")?;
        let turn_phone = conversation
            .exchange_turn_phone
            .as_deref()
            .context("exchange turns not initialized")?;

        if sender.phone != turn_phone {
            let holder = if partner.phone == turn_phone {
                &partner
            } else {
                sender
            };
            let holder_language = holder
                .learning_language
                .as_deref()
                .unwrap_or(&conversation.target_language);
            let holder_name = self.db.get_display_name(&holder.phone).await?;
            self.whatsapp
                .send_text(
                    &sender.phone,
                    &format!(
                        "It's {}'s turn — wait for them to write in {holder_language}.",
                        contact_label(&holder.phone, holder_name.as_deref())
                    ),
                )
                .await?;
            return Ok(());
        }

        let learning_language = sender
            .learning_language
            .as_deref()
            .unwrap_or(&conversation.target_language);
        let partner_learning_language = partner
            .learning_language
            .as_deref()
            .unwrap_or(&conversation.source_language);

        let history = self.db.load_history(conversation.id).await?;
        let llm = Llm::for_exchange(learning_language, partner_learning_language)?;
        let judgment = llm.validate_reply(text, &history).await?;
        if !judgment.accepted {
            self.whatsapp
                .send_text(&sender.phone, &judgment.feedback)
                .await?;
            return Ok(());
        }

        self.db
            .insert_exchange_message(conversation.id, sender, text)
            .await?;
        self.db
            .pass_exchange_turn(conversation.id, &partner.phone)
            .await?;

        let sender_name = self.db.get_display_name(&sender.phone).await?;
        let sender_label = contact_label(&sender.phone, sender_name.as_deref());
        let partner_name = self.db.get_display_name(&partner.phone).await?;
        let partner_label = contact_label(&partner.phone, partner_name.as_deref());
        let mut outgoing = format!(
            "[{sender_label}]: {text}\n\nYour turn — write in {partner_learning_language}."
        );

        let previous_active = self
            .db
            .get_active_conversation_id(&partner.phone)
            .await?;
        if previous_active.is_some() && previous_active != Some(conversation.id) {
            outgoing.push_str(&format!(
                "\n\n(Switched active chat to exchange (turns) with {sender_label}.)"
            ));
        }
        self.db
            .set_active_conversation(&partner.phone, conversation.id)
            .await?;

        self.whatsapp.send_text(&partner.phone, &outgoing).await?;
        self.whatsapp
            .send_text(
                &sender.phone,
                &format!("Message sent to {partner_label}."),
            )
            .await?;

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
