use crate::db::Db;
use crate::phone::{contact_label, looks_like_phone, normalize_phone, phones_match};
use crate::whatsapp::WhatsApp;

fn format_language_name(language: &str) -> String {
    if looks_like_phone(language) {
        "(invalid — use SET LANGUAGE)".to_string()
    } else {
        language.to_string()
    }
}

fn format_listing_line(
    index: usize,
    listing: &crate::db::ConversationListing,
    viewer_phone: &str,
) -> String {
    let language = format_language_name(&listing.target_language);
    let partner = listing
        .partner_phone
        .as_deref()
        .map(|phone| contact_label(phone, listing.partner_display_name.as_deref()))
        .unwrap_or_else(|| "unknown".into());

    let broken = listing
        .partner_phone
        .as_ref()
        .is_some_and(|partner| phones_match(partner, viewer_phone));

    let role_desc = match listing.role {
        crate::db::ParticipantRole::Teacher => format!("You teach {language}"),
        crate::db::ParticipantRole::Learner => format!("You learn {language}"),
    };

    let status = if listing.is_pending {
        " [waiting for partner]"
    } else if listing.is_active {
        " [active]"
    } else {
        ""
    };

    if broken {
        format!(
            "{index}. {role_desc} — partner: {partner} [invalid — reply CANCEL {index}]"
        )
    } else {
        format!("{index}. {role_desc} — partner: {partner}{status}")
    }
}

pub async fn handle_list(db: &Db, whatsapp: &WhatsApp, phone: &str) -> anyhow::Result<()> {
    let listings = db.list_conversations_for_phone(phone).await?;
    if listings.is_empty() {
        whatsapp
            .send_text(
                phone,
                "You don't have any conversations yet.\n\n\
                 Reply LEARNER or TEACHER to start one.",
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
    lines.push("Reply SWITCH <number> to change conversation.".into());
    lines.push("Reply CANCEL <number> to remove a pending invite.".into());
    lines.push("Reply SET LANGUAGE <name> to fix the language on the active conversation.".into());
    lines.push("Reply SET <number> <language> to fix a specific one.".into());
    lines.push("Reply LEARNER or TEACHER to start a new one.".into());

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

    db.set_active_conversation(phone, listing.conversation_id)
        .await?;

    let language = format_language_name(&listing.target_language);
    let partner = listing
        .partner_phone
        .as_deref()
        .map(|phone| contact_label(phone, listing.partner_display_name.as_deref()))
        .unwrap_or_else(|| "your partner".into());
    let role_desc = match listing.role {
        crate::db::ParticipantRole::Teacher => format!("You teach {language}"),
        crate::db::ParticipantRole::Learner => format!("You learn {language}"),
    };

    whatsapp
        .send_text(
            phone,
            &format!("Switched to {role_desc} — partner: {partner}."),
        )
        .await?;
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
            .send_text(phone, "Invalid selection. Reply LIST to see your conversations.")
            .await?;
        return Ok(());
    };

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

    let conversation_id = if let Some(index) = command.index {
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
        listing.conversation_id
    } else if let Some(listing) = listings.iter().find(|listing| listing.is_active) {
        listing.conversation_id
    } else if listings.len() == 1 {
        listings[0].conversation_id
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

    db.update_target_language(conversation_id, phone, &command.language)
        .await?;

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
             LIST — show your conversations\n\
             SWITCH <number> — change active conversation\n\
             CANCEL <number> — cancel a pending invite\n\
             SET LANGUAGE <name> — fix language on active conversation\n\
             SET <number> <language> — fix language on a specific one\n\
             LEARNER — start practicing a new language\n\
             TEACHER — teach someone a new language\n\n\
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

pub fn is_new_conversation_command(text: &str) -> Option<crate::db::ParticipantRole> {
    match text.trim().to_ascii_uppercase().as_str() {
        "LEARNER" => Some(crate::db::ParticipantRole::Learner),
        "TEACHER" => Some(crate::db::ParticipantRole::Teacher),
        _ => None,
    }
}
