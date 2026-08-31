use crate::db::{ConversationListing, ConversationMode, ConversationModeSetting, Db, SetModeError};
use crate::phone::{contact_label, looks_like_phone, normalize_phone, phones_match};
use crate::whatsapp::WhatsApp;

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
    let message = if status.is_empty() {
        format!("Switched to {role_desc} — partner: {partner}.")
    } else {
        format!("Switched to {role_desc} — partner: {partner} ({status}).")
    };

    whatsapp.send_text(phone, &message).await?;
    Ok(())
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
