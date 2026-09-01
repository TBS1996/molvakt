use crate::db::{ConversationListing, ConversationMode, ConversationModeSetting, Db, SetModeError};
use crate::phone::{contact_label, looks_like_phone, normalize_phone, phones_match};
use crate::whatsapp::WhatsApp;
use anyhow::Context;

fn format_language_name(language: &str) -> String {
    if looks_like_phone(language) {
        "(invalid — use SET LANGUAGE)".to_string()
    } else {
        language.to_string()
    }
}

pub fn listing_is_broken(listing: &ConversationListing, viewer_phone: &str) -> bool {
    listing
        .partner_phone
        .as_ref()
        .is_some_and(|partner| phones_match(partner, viewer_phone))
}

pub fn format_listing_partner(listing: &ConversationListing) -> String {
    listing
        .partner_phone
        .as_deref()
        .map(|phone| contact_label(phone, listing.partner_display_name.as_deref()))
        .unwrap_or_else(|| "unknown".into())
}

pub fn format_listing_role_desc(listing: &ConversationListing) -> String {
    let language = format_language_name(&listing.target_language);
    if listing.mode == ConversationMode::Exchange {
        format_exchange_role_desc(listing, false)
    } else if listing.mode == ConversationMode::ExchangeTurns {
        format_exchange_role_desc(listing, true)
    } else {
        match listing.role {
            crate::db::ParticipantRole::Teacher => format!("You teach {language}"),
            crate::db::ParticipantRole::Learner => format!("You learn {language}"),
        }
    }
}

pub fn listing_awaits_user_reply(listing: &ConversationListing, viewer_phone: &str) -> bool {
    if listing.is_pending || listing_is_broken(listing, viewer_phone) {
        return false;
    }
    match listing.turn {
        Some(crate::db::ConversationTurnStatus::YourTurnToReply) => true,
        Some(crate::db::ConversationTurnStatus::YourTurnToSend) => {
            listing.mode == ConversationMode::ExchangeTurns
                || (listing.mode == ConversationMode::Tutor
                    && listing.role == crate::db::ParticipantRole::Teacher)
        }
        _ => false,
    }
}

pub fn listing_should_show_partner_last_message(listing: &ConversationListing) -> bool {
    match listing.turn {
        Some(crate::db::ConversationTurnStatus::YourTurnToReply) => true,
        Some(crate::db::ConversationTurnStatus::YourTurnToSend) => {
            listing.mode == ConversationMode::ExchangeTurns
        }
        _ => false,
    }
}

pub const PING_PARTNER_PREFIX: &str = "ping_";
pub const PING_MIN_MINUTES: i64 = 30;

pub fn parse_ping_button(text: &str) -> Option<i64> {
    text.strip_prefix(PING_PARTNER_PREFIX)
        .and_then(|id| id.parse().ok())
}

pub fn ping_button_id(conversation_id: i64) -> String {
    format!("{PING_PARTNER_PREFIX}{conversation_id}")
}

pub async fn can_ping_partner(
    db: &Db,
    listing: &ConversationListing,
    viewer_phone: &str,
) -> anyhow::Result<bool> {
    if listing.is_pending || listing_is_broken(listing, viewer_phone) {
        return Ok(false);
    }

    // You sent last and are still waiting for them to respond.
    if !db
        .viewer_sent_last_message(listing.conversation_id, viewer_phone)
        .await?
    {
        return Ok(false);
    }

    let still_their_turn = matches!(
        listing.turn,
        Some(crate::db::ConversationTurnStatus::WaitingForReply)
            | Some(crate::db::ConversationTurnStatus::WaitingForMessage)
    );
    if !still_their_turn {
        return Ok(false);
    }

    let Some(minutes) = db
        .minutes_since_viewer_sent_message(listing.conversation_id, viewer_phone)
        .await?
    else {
        return Ok(false);
    };

    Ok(minutes >= PING_MIN_MINUTES)
}

pub async fn send_waiting_for_partner_notice(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    conversation_id: i64,
    body: &str,
) -> anyhow::Result<()> {
    let listings = db.list_conversations_for_phone(phone).await?;
    let Some(listing) = listings
        .iter()
        .find(|listing| listing.conversation_id == conversation_id)
    else {
        whatsapp.send_text(phone, body).await?;
        return Ok(());
    };

    if can_ping_partner(db, listing, phone).await? {
        let ping_id = ping_button_id(conversation_id);
        whatsapp
            .send_review_choice_list(
                phone,
                body,
                "Options",
                &[(ping_id.as_str(), "Ping them", "")],
            )
            .await?;
    } else {
        whatsapp.send_text(phone, body).await?;
    }

    Ok(())
}

pub async fn handle_ping(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    conversation_id: i64,
) -> anyhow::Result<()> {
    let listings = db.list_conversations_for_phone(phone).await?;
    let Some(listing) = listings
        .iter()
        .find(|listing| listing.conversation_id == conversation_id)
    else {
        whatsapp
            .send_text(phone, "That chat is no longer available.")
            .await?;
        return Ok(());
    };

    if !can_ping_partner(db, listing, phone).await? {
        whatsapp
            .send_text(
                phone,
                "You can't ping right now — it's your turn, they already replied, or it's been less than 30 minutes since you sent.",
            )
            .await?;
        return Ok(());
    }

    let partner_phone = listing
        .partner_phone
        .as_deref()
        .context("no partner in conversation")?;
    let sender_name = db.get_display_name(phone).await?;
    let sender_label = contact_label(phone, sender_name.as_deref());
    whatsapp
        .send_text(
            partner_phone,
            &format!("{sender_label} pinged you to reply to their message."),
        )
        .await?;

    let partner_name = db.get_display_name(partner_phone).await?;
    whatsapp
        .send_text(
            phone,
            &format!(
                "Reminder sent to {}.",
                contact_label(partner_phone, partner_name.as_deref())
            ),
        )
        .await?;

    Ok(())
}

pub fn format_listing_status_text(listing: &ConversationListing) -> String {
    if listing.is_pending {
        "waiting for partner".to_string()
    } else if let Some(turn) = listing.turn {
        turn.label().to_string()
    } else {
        String::new()
    }
}

pub fn format_listing_menu_description(
    listing: &ConversationListing,
    viewer_phone: &str,
) -> String {
    if listing_is_broken(listing, viewer_phone) {
        return "Invalid invite".to_string();
    }

    let mut description = format_listing_role_desc(listing);
    let status = format_listing_status_text(listing);
    if !status.is_empty() {
        description.push_str(", ");
        description.push_str(&status);
    }
    if description.len() > 72 {
        description.truncate(72);
    }
    description
}

fn format_active_chat_summary(listing: &ConversationListing, viewer_phone: &str) -> String {
    let partner = format_listing_partner(listing);
    if listing_is_broken(listing, viewer_phone) {
        return format!("Current chat: {partner}\nInvalid invite — cancel it from the menu.");
    }
    if listing.is_pending {
        return format!("Current chat: {partner}\nWaiting for partner to accept.");
    }

    let role_desc = format_listing_role_desc(listing);
    let status = format_listing_status_text(listing);
    if status.is_empty() {
        format!("Current chat: {partner}\n{role_desc}")
    } else {
        format!("Current chat: {partner}\n{role_desc}\n{status}")
    }
}

pub async fn format_menu_body(db: &Db, phone: &str) -> anyhow::Result<String> {
    let listings = db.list_conversations_for_phone(phone).await?;
    if listings.is_empty() {
        return Ok("No chats yet — start one below.".to_string());
    }

    let viewer_phone = normalize_phone(phone);
    if let Some(listing) = listings.iter().find(|listing| listing.is_active) {
        return Ok(format_active_chat_summary(listing, &viewer_phone));
    }

    if listings.len() == 1 {
        return Ok(format!(
            "{}\n\n(This is your only chat — it becomes active when you message.)",
            format_active_chat_summary(&listings[0], &viewer_phone)
        ));
    }

    Ok("No active chat — use Switch chat below.".to_string())
}

fn format_listing_status(listing: &ConversationListing) -> String {
    let mut tags: Vec<String> = Vec::new();
    if listing.is_active {
        tags.push("active".into());
    }
    if listing.is_pending {
        tags.push("waiting for partner".into());
    } else if listing.mode == ConversationMode::ExchangeTurns {
        if let Some(language) = &listing.exchange_active_language {
            match listing.turn {
                Some(crate::db::ConversationTurnStatus::YourTurnToSend) => {
                    tags.push(format!("your turn — write in {language}"));
                }
                Some(crate::db::ConversationTurnStatus::WaitingForMessage) => {
                    tags.push(format!("waiting — current language: {language}"));
                }
                _ => tags.push(format!("current language: {language}")),
            }
        }
    } else if let Some(turn) = listing.turn {
        tags.push(turn.label().into());
    }
    if tags.is_empty() {
        String::new()
    } else {
        format!(" [{}]", tags.join(", "))
    }
}

fn format_exchange_role_desc(listing: &ConversationListing, turns: bool) -> String {
    let your_language = listing
        .learning_language
        .as_deref()
        .map(format_language_name)
        .unwrap_or_else(|| "unknown".into());
    let partner_language = listing
        .partner_learning_language
        .as_deref()
        .map(format_language_name)
        .unwrap_or_else(|| "?".into());
    let label = if turns {
        "Exchange (turns)"
    } else {
        "Exchange"
    };
    if turns {
        format!(
            "{label} — you learn {your_language}, partner learns {partner_language}; alternate languages each round"
        )
    } else {
        format!(
            "{label} — you learn {your_language}, partner learns {partner_language}; always write in your language"
        )
    }
}

pub fn format_listing_line(
    index: usize,
    listing: &ConversationListing,
    viewer_phone: &str,
) -> String {
    let partner = format_listing_partner(listing);
    let role_desc = format_listing_role_desc(listing);
    let status = format_listing_status(listing);

    if listing_is_broken(listing, viewer_phone) {
        format!(
            "{index}. {role_desc} — partner: {partner} [invalid — reply CANCEL {index}]"
        )
    } else {
        format!("{index}. {role_desc} — partner: {partner}{status}")
    }
}

pub fn format_chat_label(
    role: crate::db::ParticipantRole,
    language: &str,
    partner_phone: &str,
    partner_display_name: Option<&str>,
) -> String {
    let language = format_language_name(language);
    let partner = contact_label(partner_phone, partner_display_name);
    match role {
        crate::db::ParticipantRole::Learner => format!("{language} with {partner}"),
        crate::db::ParticipantRole::Teacher => format!("teaching {language} to {partner}"),
    }
}

pub async fn handle_list(db: &Db, whatsapp: &WhatsApp, phone: &str) -> anyhow::Result<()> {
    let listings = db.list_conversations_for_phone(phone).await?;
    if listings.is_empty() {
        whatsapp
            .send_text(
                phone,
                "You don't have any conversations yet.\n\n\
                 Reply LEARNER, TEACHER, EXCHANGE, or EXCHANGE-TURNS to start one.\n\
                 (Or open the menu if you haven't set up yet.)",
            )
            .await?;
        return Ok(());
    }

    let viewer_phone = normalize_phone(phone);
    let mut lines = vec!["Your conversations:".to_string()];
    for (index, listing) in listings.iter().enumerate() {
        lines.push(format_listing_line(index + 1, listing, &viewer_phone));
    }

    lines.push(String::new());
    lines.push("Reply MENU for actions, or use text commands:".into());
    lines.push("SWITCH <number>, SET MODE <number> <mode>, CANCEL <number>.".into());
    lines.push("SET LANGUAGE <name>, or SET <number> <language>.".into());
    lines.push("LEARNER, TEACHER, EXCHANGE, or EXCHANGE-TURNS to start a new one.".into());

    whatsapp.send_text(phone, &lines.join("\n")).await?;
    Ok(())
}

pub async fn handle_switch(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    selection: usize,
) -> anyhow::Result<()> {
    let listings = db.list_conversations_for_phone(phone).await?;
    let Some(listing) = listings.get(selection.saturating_sub(1)) else {
        whatsapp
            .send_text(
                phone,
                &format!(
                    "Invalid selection. Reply LIST to see your {} conversation(s).",
                    listings.len()
                ),
            )
            .await?;
        return Ok(());
    };

    let viewer_phone = normalize_phone(phone);
    if listing_is_broken(listing, &viewer_phone) {
        whatsapp
            .send_text(
                phone,
                "That conversation is invalid. Reply CANCEL to remove it.",
            )
            .await?;
        return Ok(());
    }

    if listing.is_pending {
        whatsapp
            .send_text(
                phone,
                "That conversation is still waiting for your partner to accept the invite.",
            )
            .await?;
        return Ok(());
    }

    db.set_active_conversation(phone, listing.conversation_id)
        .await?;

    let role_desc = format_listing_role_desc(listing);
    let partner = format_listing_partner(listing);
    let status = format_listing_status_text(listing);
    let mut message = if status.is_empty() {
        format!("Switched to {role_desc} — partner: {partner}.")
    } else {
        format!("Switched to {role_desc} — partner: {partner} ({status}).")
    };

    if listing_should_show_partner_last_message(listing) {
        if let Some(last_message) = db
            .last_partner_message(
                listing.conversation_id,
                listing.mode,
                phone,
                listing.role,
            )
            .await?
        {
            message.push_str(&format!(
                "\n\nLast from {partner}:\n{}",
                truncate_for_display(&last_message, 400)
            ));
        }
    }

    whatsapp.send_text(phone, &message).await?;
    Ok(())
}

fn truncate_for_display(text: &str, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max_len).collect();
    format!("{truncated}…")
}

pub struct SetModeCommand {
    pub index: Option<usize>,
    pub mode: ConversationModeSetting,
}

pub async fn handle_set_mode(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    command: SetModeCommand,
) -> anyhow::Result<()> {
    let listings = db.list_conversations_for_phone(phone).await?;
    let listing = if let Some(index) = command.index {
        let Some(listing) = listings.get(index.saturating_sub(1)) else {
            whatsapp
                .send_text(
                    phone,
                    &format!(
                        "Invalid selection. Reply LIST to see your {} conversation(s).",
                        listings.len()
                    ),
                )
                .await?;
            return Ok(());
        };
        listing
    } else if let Some(listing) = listings.iter().find(|listing| listing.is_active) {
        listing
    } else if listings.len() == 1 {
        &listings[0]
    } else {
        whatsapp
            .send_text(
                phone,
                "You have multiple conversations. Use SET MODE <number> <mode>, e.g. SET MODE 1 exchange.\n\
                 Reply LIST to see the numbers.",
            )
            .await?;
        return Ok(());
    };

    let viewer_phone = normalize_phone(phone);
    let broken = listing
        .partner_phone
        .as_ref()
        .is_some_and(|partner| phones_match(partner, &viewer_phone));

    if broken {
        whatsapp
            .send_text(
                phone,
                "That conversation is invalid. Reply CANCEL to remove it.",
            )
            .await?;
        return Ok(());
    }

    if listing.is_pending {
        whatsapp
            .send_text(
                phone,
                "That conversation is still waiting for your partner to accept the invite.",
            )
            .await?;
        return Ok(());
    }

    match db
        .apply_conversation_mode(listing.conversation_id, phone, command.mode)
        .await
    {
        Ok((you, partner)) => {
            let partner_name = db.get_display_name(&partner.phone).await?;
            let your_name = db.get_display_name(phone).await?;
            let partner_label = contact_label(&partner.phone, partner_name.as_deref());
            let your_label = contact_label(phone, your_name.as_deref());

            match command.mode {
                ConversationModeSetting::Exchange => {
                    let your_language = you
                        .learning_language
                        .as_deref()
                        .map(format_language_name)
                        .unwrap_or_else(|| "not set".into());
                    let partner_language = partner
                        .learning_language
                        .as_deref()
                        .map(format_language_name)
                        .unwrap_or_else(|| "not set".into());

                    whatsapp
                        .send_text(
                            phone,
                            &format!(
                                "Switched to exchange mode with {partner_label}. \
                                 You learn {your_language} — always write in that language."
                            ),
                        )
                        .await?;

                    whatsapp
                        .send_text(
                            &partner.phone,
                            &format!(
                                "{your_label} switched this chat to exchange mode. \
                                 You learn {partner_language} — always write in that language."
                            ),
                        )
                        .await?;
                }
                ConversationModeSetting::ExchangeTurns => {
                    let your_language = you
                        .learning_language
                        .as_deref()
                        .map(format_language_name)
                        .unwrap_or_else(|| "not set".into());
                    let partner_language = partner
                        .learning_language
                        .as_deref()
                        .map(format_language_name)
                        .unwrap_or_else(|| "not set".into());

                    whatsapp
                        .send_text(
                            phone,
                            &format!(
                                "Switched to turn-based exchange with {partner_label}. \
                                 You learn {your_language}, they learn {partner_language}. \
                                 Take turns — both write in one language, then both write in the other."
                            ),
                        )
                        .await?;

                    whatsapp
                        .send_text(
                            &partner.phone,
                            &format!(
                                "{your_label} switched this chat to turn-based exchange. \
                                 You learn {partner_language}, they learn {your_language}. \
                                 Take turns in the shared language each round — you go first."
                            ),
                        )
                        .await?;
                }
                ConversationModeSetting::Teacher | ConversationModeSetting::Learner => {
                    let conversation = db.get_conversation(listing.conversation_id).await?;
                    let language = format_language_name(&conversation.target_language);
                    whatsapp
                        .send_text(
                            phone,
                            &format!(
                                "You are now the {} for {language} with {partner_label}.",
                                you.role.label(),
                            ),
                        )
                        .await?;
                    whatsapp
                        .send_text(
                            &partner.phone,
                            &format!(
                                "{your_label} switched mode. You are now the {} for {language}.",
                                partner.role.label(),
                            ),
                        )
                        .await?;
                }
            }
        }
        Err(error) if error.downcast_ref::<SetModeError>() == Some(&SetModeError::ActiveExchange) => {
            whatsapp
                .send_text(
                    phone,
                    "Can't change mode while a message is in progress. Wait until the learner finishes their current reply.",
                )
                .await?;
        }
        Err(error) if error.downcast_ref::<SetModeError>() == Some(&SetModeError::NotComplete) => {
            whatsapp
                .send_text(phone, "That conversation isn't ready for a mode change yet.")
                .await?;
        }
        Err(error) if error.downcast_ref::<SetModeError>() == Some(&SetModeError::NotParticipant) => {
            whatsapp
                .send_text(phone, "You aren't part of that conversation.")
                .await?;
        }
        Err(error) => return Err(error),
    }

    Ok(())
}

pub async fn handle_cancel(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    selection: usize,
) -> anyhow::Result<()> {
    let listings = db.list_conversations_for_phone(phone).await?;
    let Some(listing) = listings.get(selection.saturating_sub(1)) else {
        whatsapp
            .send_text(phone, "Invalid selection. Reply MENU or LIST to see your conversations.")
            .await?;
        return Ok(());
    };

    handle_cancel_listing(db, whatsapp, phone, listing).await
}

pub async fn handle_cancel_listing(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    listing: &ConversationListing,
) -> anyhow::Result<()> {
    let viewer_phone = normalize_phone(phone);
    let broken = listing
        .partner_phone
        .as_ref()
        .is_some_and(|partner| phones_match(partner, &viewer_phone));

    if !listing.is_pending && !broken {
        whatsapp
            .send_text(phone, "Only pending or invalid invites can be cancelled.")
            .await?;
        return Ok(());
    }

    db.delete_conversation(listing.conversation_id).await?;
    whatsapp
        .send_text(phone, "Pending invite cancelled.")
        .await?;
    Ok(())
}

pub struct SetLanguageCommand {
    pub index: Option<usize>,
    pub language: String,
}

pub async fn handle_set_language(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    command: SetLanguageCommand,
) -> anyhow::Result<()> {
    if command.language.is_empty() {
        whatsapp
            .send_text(phone, "Please include a language, e.g. SET LANGUAGE Norwegian")
            .await?;
        return Ok(());
    }
    if looks_like_phone(&command.language) {
        whatsapp
            .send_text(
                phone,
                "That looks like a phone number, not a language.\n\
                 Example: SET LANGUAGE Norwegian",
            )
            .await?;
        return Ok(());
    }

    let listings = db.list_conversations_for_phone(phone).await?;
    if listings.is_empty() {
        whatsapp
            .send_text(phone, "You don't have any conversations yet.")
            .await?;
        return Ok(());
    }

    let (conversation_id, per_participant_language) = if let Some(index) = command.index {
        let Some(listing) = listings.get(index.saturating_sub(1)) else {
            whatsapp
                .send_text(
                    phone,
                    &format!(
                        "Invalid selection. Reply LIST to see your {} conversation(s).",
                        listings.len()
                    ),
                )
                .await?;
            return Ok(());
        };
        (
            listing.conversation_id,
            listing.mode.is_exchange(),
        )
    } else if let Some(listing) = listings.iter().find(|listing| listing.is_active) {
        (listing.conversation_id, listing.mode.is_exchange())
    } else if listings.len() == 1 {
        (
            listings[0].conversation_id,
            listings[0].mode.is_exchange(),
        )
    } else {
        whatsapp
            .send_text(
                phone,
                "You have multiple conversations. Use SET <number> <language>, e.g. SET 1 Norwegian.\n\
                 Reply LIST to see the numbers.",
            )
            .await?;
        return Ok(());
    };

    if per_participant_language {
        db.update_learning_language(conversation_id, phone, &command.language)
            .await?;

        let conversation = db.get_conversation(conversation_id).await?;
        if conversation.mode == ConversationMode::ExchangeTurns {
            let (lang_a, lang_b) = db.exchange_language_pair(&conversation).await?;
            let active = conversation.exchange_active_language();
            if active != lang_a && active != lang_b {
                let starter = conversation
                    .exchange_turn_phone
                    .as_deref()
                    .unwrap_or(phone);
                db.init_exchange_round_state(conversation_id, starter, &lang_a, true)
                    .await?;
            }
        }
    } else {
        db.update_target_language(conversation_id, phone, &command.language)
            .await?;
    }

    whatsapp
        .send_text(
            phone,
            &format!("Language updated to {}.", command.language),
        )
        .await?;
    Ok(())
}

pub async fn handle_help(whatsapp: &WhatsApp, phone: &str) -> anyhow::Result<()> {
    whatsapp
        .send_text(
            phone,
            "molvakt commands:\n\n\
             MENU — open the action menu (recommended)\n\
             Review vocab — flashcards from your chats (also in MENU)\n\
             LIST — show your conversations\n\
             SWITCH <number> — change active conversation\n\
             SET MODE <number> teacher|learner|exchange|exchange-turns — change conversation mode\n\
             CANCEL <number> — cancel a pending invite\n\
             SET LANGUAGE <name> — fix language on active conversation\n\
             SET TIMEZONE <region> — morning reminder time (e.g. Europe/Oslo)\n\
             SET <number> <language> — fix language on a specific one\n\
             LEARNER / TEACHER / EXCHANGE / EXCHANGE-TURNS — start a new conversation\n\
             (New users: message the bot to open the setup menu)\n\n\
             In an active conversation, just message normally.",
        )
        .await?;
    Ok(())
}

pub fn parse_set_language(text: &str) -> Option<SetLanguageCommand> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < 2 || !words[0].eq_ignore_ascii_case("SET") {
        return None;
    }
    if words.len() >= 2 && words[1].eq_ignore_ascii_case("MODE") {
        return None;
    }

    if words.len() >= 3 && words[1].eq_ignore_ascii_case("LANGUAGE") {
        return Some(SetLanguageCommand {
            index: None,
            language: words[2..].join(" "),
        });
    }

    let index = words[1].parse().ok()?;
    if words.len() < 3 {
        return None;
    }

    Some(SetLanguageCommand {
        index: Some(index),
        language: words[2..].join(" "),
    })
}

pub fn parse_set_mode(text: &str) -> Option<SetModeCommand> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < 3 || !words[0].eq_ignore_ascii_case("SET") || !words[1].eq_ignore_ascii_case("MODE")
    {
        return None;
    }

    let (index, mode_index) = if let Ok(index) = words[2].parse::<usize>() {
        (Some(index), 3)
    } else {
        (None, 2)
    };

    let mode = match words.get(mode_index)?.to_ascii_lowercase().as_str() {
        "teacher" => ConversationModeSetting::Teacher,
        "learner" => ConversationModeSetting::Learner,
        "exchange" => ConversationModeSetting::Exchange,
        "exchange-turns" | "exchange_turns" | "turns" => ConversationModeSetting::ExchangeTurns,
        _ => return None,
    };

    Some(SetModeCommand { index, mode })
}

pub fn parse_switch_selection(text: &str) -> Option<usize> {
    parse_numbered_command(text, "SWITCH")
}

pub fn parse_cancel_selection(text: &str) -> Option<usize> {
    parse_numbered_command(text, "CANCEL")
}

fn parse_numbered_command(text: &str, command: &str) -> Option<usize> {
    let text = text.trim();
    let rest = text.strip_prefix(command)?.trim();
    rest.parse().ok()
}

pub fn is_list_command(text: &str) -> bool {
    matches!(text.trim().to_ascii_uppercase().as_str(), "LIST" | "CONVERSATIONS")
}

pub fn is_help_command(text: &str) -> bool {
    matches!(text.trim().to_ascii_uppercase().as_str(), "HELP" | "COMMANDS")
}

pub enum StartConversationCommand {
    Tutor(crate::db::ParticipantRole),
    Exchange(crate::db::ConversationMode),
}

pub fn parse_start_conversation_command(text: &str) -> Option<StartConversationCommand> {
    match text.trim().to_ascii_uppercase().as_str() {
        "LEARNER" => Some(StartConversationCommand::Tutor(
            crate::db::ParticipantRole::Learner,
        )),
        "TEACHER" => Some(StartConversationCommand::Tutor(
            crate::db::ParticipantRole::Teacher,
        )),
        "EXCHANGE" => Some(StartConversationCommand::Exchange(
            crate::db::ConversationMode::Exchange,
        )),
        "EXCHANGE-TURNS" | "EXCHANGE_TURNS" | "TURNS" => Some(StartConversationCommand::Exchange(
            crate::db::ConversationMode::ExchangeTurns,
        )),
        _ => None,
    }
}
