use crate::db::{
    ConversationInvite, Db, OnboardingData, OnboardingStep, ParticipantRole,
};
use anyhow::Context;
use crate::phone::{display_phone, looks_like_phone, normalize_phone, phones_match};
use crate::whatsapp::WhatsApp;

pub async fn start_new_conversation(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    role: ParticipantRole,
) -> anyhow::Result<()> {
    db.clear_onboarding_session(phone).await?;

    let mut data = OnboardingData::default();
    data.role = Some(role);
    db.save_onboarding_session(phone, OnboardingStep::EnterPartnerPhone, &data)
        .await?;

    let prompt = match role {
        ParticipantRole::Learner => {
            "Send your teacher's phone number with country code (e.g. +4791234567)."
        }
        ParticipantRole::Teacher => {
            "Send your learner's phone number with country code (e.g. +14155551234)."
        }
    };
    whatsapp.send_text(phone, prompt).await?;
    Ok(())
}

pub async fn handle_new_or_onboarding_user(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    text: &str,
) -> anyhow::Result<()> {
    if let Some((step, data)) = db.load_onboarding_session(phone).await? {
        return continue_onboarding(db, whatsapp, phone, text, step, data).await;
    }

    match text.trim().to_ascii_uppercase().as_str() {
        "LEARNER" => {
            let mut data = OnboardingData::default();
            data.role = Some(ParticipantRole::Learner);
            db.save_onboarding_session(phone, OnboardingStep::EnterPartnerPhone, &data)
                .await?;
            whatsapp
                .send_text(
                    phone,
                    "Send your teacher's phone number with country code (e.g. +4791234567).",
                )
                .await?;
        }
        "TEACHER" => {
            let mut data = OnboardingData::default();
            data.role = Some(ParticipantRole::Teacher);
            db.save_onboarding_session(phone, OnboardingStep::EnterPartnerPhone, &data)
                .await?;
            whatsapp
                .send_text(
                    phone,
                    "Send your learner's phone number with country code (e.g. +14155551234).",
                )
                .await?;
        }
        _ => {
            send_welcome(whatsapp, phone).await?;
            db.save_onboarding_session(phone, OnboardingStep::Welcome, &OnboardingData::default())
                .await?;
        }
    }

    Ok(())
}

pub async fn handle_invite_response(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    text: &str,
    invite: ConversationInvite,
) -> anyhow::Result<()> {
    match text.trim().to_ascii_uppercase().as_str() {
        "ACCEPT" => accept_invite(db, whatsapp, phone, invite).await,
        "DECLINE" => decline_invite(db, whatsapp, phone, invite).await,
        _ => {
            let conversation = db.get_conversation(invite.conversation_id).await?;
            let inviter = display_phone(&invite.inviter_phone);
            let invitee_role = invite.inviter_role.opposite();
            let role_label = match invitee_role {
                ParticipantRole::Teacher => "teacher",
                ParticipantRole::Learner => "learner",
            };
            whatsapp
                .send_text(
                    phone,
                    &format!(
                        "You have a pending invite from {inviter} to practice {} as their {role_label}.\n\n\
                         Reply ACCEPT or DECLINE.",
                        conversation.target_language
                    ),
                )
                .await?;
            Ok(())
        }
    }
}

async fn continue_onboarding(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    text: &str,
    step: OnboardingStep,
    mut data: OnboardingData,
) -> anyhow::Result<()> {
    match step {
        OnboardingStep::Welcome => match text.trim().to_ascii_uppercase().as_str() {
            "LEARNER" => {
                data.role = Some(ParticipantRole::Learner);
                db.save_onboarding_session(phone, OnboardingStep::EnterPartnerPhone, &data)
                    .await?;
                whatsapp
                    .send_text(
                        phone,
                        "Send your teacher's phone number with country code (e.g. +4791234567).",
                    )
                    .await?;
            }
            "TEACHER" => {
                data.role = Some(ParticipantRole::Teacher);
                db.save_onboarding_session(phone, OnboardingStep::EnterPartnerPhone, &data)
                    .await?;
                whatsapp
                    .send_text(
                        phone,
                        "Send your learner's phone number with country code (e.g. +14155551234).",
                    )
                    .await?;
            }
            _ => send_welcome(whatsapp, phone).await?,
        },
        OnboardingStep::EnterPartnerPhone => {
            let partner_phone = normalize_phone(text);
            if partner_phone.len() < 8 {
                whatsapp
                    .send_text(
                        phone,
                        "That doesn't look like a valid phone number. Include country code, e.g. +4791234567.",
                    )
                    .await?;
                return Ok(());
            }
            if partner_phone == normalize_phone(phone) {
                whatsapp
                    .send_text(phone, "You can't pair with your own number. Send your partner's phone.")
                    .await?;
                return Ok(());
            }

            data.partner_phone = Some(partner_phone);
            db.save_onboarding_session(phone, OnboardingStep::EnterTargetLanguage, &data)
                .await?;
            whatsapp
                .send_text(phone, "What language will you practice? (e.g. Norwegian)")
                .await?;
        }
        OnboardingStep::EnterTargetLanguage => {
            let target_language = text.trim();
            if target_language.is_empty() {
                whatsapp
                    .send_text(phone, "Please send the language name, e.g. Norwegian.")
                    .await?;
                return Ok(());
            }
            if looks_like_phone(target_language) {
                whatsapp
                    .send_text(
                        phone,
                        "That looks like a phone number, not a language.\n\
                         Send the language name, e.g. Norwegian.",
                    )
                    .await?;
                return Ok(());
            }

            data.target_language = Some(target_language.to_string());
            if data.source_language.is_none() {
                data.source_language = Some(default_source_language());
            }

            send_invite(db, whatsapp, phone, &data).await?;
            db.clear_onboarding_session(phone).await?;
        }
    }

    Ok(())
}

async fn send_invite(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    data: &OnboardingData,
) -> anyhow::Result<()> {
    let role = data.role.context("missing role during invite")?;
    let partner_phone = data
        .partner_phone
        .as_ref()
        .context("missing partner phone during invite")?;
    let target_language = data
        .target_language
        .as_ref()
        .context("missing target language during invite")?;
    let source_language = data
        .source_language
        .clone()
        .unwrap_or_else(default_source_language);

    if phones_match(phone, partner_phone) {
        whatsapp
            .send_text(phone, "You can't pair with your own number. Send your partner's phone.")
            .await?;
        return Ok(());
    }

    if db
        .find_complete_conversation_between(phone, partner_phone)
        .await?
        .is_some()
    {
        whatsapp
            .send_text(
                phone,
                &format!(
                    "You're already connected with {}. Reply LIST to see your conversations.",
                    display_phone(partner_phone)
                ),
            )
            .await?;
        return Ok(());
    }

    if let Some(invite) = db
        .find_pending_invite_between(phone, partner_phone)
        .await?
    {
        let message = if phones_match(&invite.inviter_phone, phone) {
            format!(
                "You already have a pending invite with {}. Waiting for them to accept.",
                display_phone(partner_phone)
            )
        } else {
            format!(
                "{} already invited you! Reply ACCEPT or DECLINE to their invite first.",
                display_phone(partner_phone)
            )
        };
        whatsapp.send_text(phone, &message).await?;
        return Ok(());
    }

    let conversation = db
        .create_conversation(target_language, &source_language)
        .await?;
    let inviter = db
        .register_participant(conversation.id, phone, role)
        .await?;

    if role == ParticipantRole::Learner {
        db.init_learner_session(inviter.id).await?;
    }

    let invite = db
        .create_invite(conversation.id, phone, partner_phone, role)
        .await?;

    let inviter_display = display_phone(phone);
    let invitee_role = role.opposite();
    let role_label = match invitee_role {
        ParticipantRole::Teacher => "teacher",
        ParticipantRole::Learner => "learner",
    };

    whatsapp
        .send_text(
            partner_phone,
            &format!(
                "{inviter_display} wants to practice {target_language} with you as their {role_label}.\n\n\
                 Reply ACCEPT or DECLINE."
            ),
        )
        .await?;

    whatsapp
        .send_text(
            phone,
            "Invite sent! Waiting for them to accept — you'll get a message when the conversation is ready.",
        )
        .await?;

    println!(
        "invite {} created: conversation {} from {} to {}",
        invite.id, conversation.id, phone, partner_phone
    );

    Ok(())
}

async fn accept_invite(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    invite: ConversationInvite,
) -> anyhow::Result<()> {
    if normalize_phone(phone) != invite.invitee_phone {
        anyhow::bail!("invite phone mismatch");
    }

    if db
        .find_complete_conversation_between(&invite.inviter_phone, phone)
        .await?
        .is_some()
    {
        whatsapp
            .send_text(phone, "You're already connected with this person.")
            .await?;
        db.update_invite_status(invite.id, crate::db::InviteStatus::Declined)
            .await?;
        db.delete_conversation(invite.conversation_id).await?;
        return Ok(());
    }

    let conversation = db.get_conversation(invite.conversation_id).await?;
    let invitee_role = invite.inviter_role.opposite();
    let invitee = db
        .register_participant(invite.conversation_id, phone, invitee_role)
        .await?;

    if invitee_role == ParticipantRole::Learner {
        db.init_learner_session(invitee.id).await?;
    }

    db.update_invite_status(invite.id, crate::db::InviteStatus::Accepted)
        .await?;
    db.set_active_conversation(&invite.inviter_phone, invite.conversation_id)
        .await?;

    match invitee_role {
        ParticipantRole::Teacher => {
            whatsapp
                .send_text(
                    phone,
                    &format!(
                        "You're connected! Send messages in {} and molvakt will forward them to your learner.\n\n\
                         Reply LIST to see all your conversations.",
                        conversation.target_language
                    ),
                )
                .await?;
        }
        ParticipantRole::Learner => {
            whatsapp
                .send_text(
                    phone,
                    &format!(
                        "You're connected! You'll practice {} here — reply when your teacher messages you.\n\n\
                         Reply LIST to see all your conversations.",
                        conversation.target_language
                    ),
                )
                .await?;
        }
    }

    whatsapp
        .send_text(
            &invite.inviter_phone,
            &format!(
                "You're connected to {}! Reply LIST to see all conversations or just start messaging.",
                display_phone(phone)
            ),
        )
        .await?;

    Ok(())
}

async fn decline_invite(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    invite: ConversationInvite,
) -> anyhow::Result<()> {
    db.update_invite_status(invite.id, crate::db::InviteStatus::Declined)
        .await?;
    db.delete_conversation(invite.conversation_id).await?;

    whatsapp
        .send_text(phone, "Invite declined.")
        .await?;
    whatsapp
        .send_text(
            &invite.inviter_phone,
            &format!(
                "{} declined your invite. You can start over by sending LEARNER or TEACHER.",
                display_phone(phone)
            ),
        )
        .await?;

    Ok(())
}

async fn send_welcome(whatsapp: &WhatsApp, phone: &str) -> anyhow::Result<()> {
    whatsapp
        .send_text(
            phone,
            "Welcome to molvakt!\n\n\
             Reply LEARNER if you're practicing a language.\n\
             Reply TEACHER if you're the native speaker.\n\n\
             Already set up? Reply LIST or HELP.",
        )
        .await
}

fn default_source_language() -> String {
    std::env::var("MOLVAKT_SOURCE_LANGUAGE").unwrap_or_else(|_| "English".into())
}
