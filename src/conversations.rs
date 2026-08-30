use crate::db::Db;
use crate::phone::display_phone;
use crate::whatsapp::WhatsApp;

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

    let mut lines = vec!["Your conversations:".to_string()];
    for (index, listing) in listings.iter().enumerate() {
        let partner = listing
            .partner_phone
            .as_deref()
            .map(display_phone)
            .unwrap_or_else(|| "partner".into());
        let role = match listing.role {
            crate::db::ParticipantRole::Teacher => "teacher",
            crate::db::ParticipantRole::Learner => "learner",
        };
        let status = if listing.is_pending {
            "waiting for partner".to_string()
        } else if listing.is_active {
            "active".to_string()
        } else {
            String::new()
        };
        let status_suffix = if status.is_empty() {
            String::new()
        } else {
            format!(" [{status}]")
        };
        lines.push(format!(
            "{}. {} ({role}) with {partner}{status_suffix}",
            index + 1,
            listing.target_language
        ));
    }

    lines.push(String::new());
    lines.push("Reply SWITCH <number> to change conversation.".into());
    lines.push("Reply CANCEL <number> to remove a pending invite.".into());
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

    let partner = listing
        .partner_phone
        .as_deref()
        .map(display_phone)
        .unwrap_or_else(|| "your partner".into());
    let role = match listing.role {
        crate::db::ParticipantRole::Teacher => "teacher",
        crate::db::ParticipantRole::Learner => "learner",
    };

    whatsapp
        .send_text(
            phone,
            &format!(
                "Switched to {} ({role}) with {partner}.",
                listing.target_language
            ),
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

    if !listing.is_pending {
        whatsapp
            .send_text(phone, "Only pending invites can be cancelled. Pick one marked [waiting for partner].")
            .await?;
        return Ok(());
    }

    db.delete_conversation(listing.conversation_id).await?;
    whatsapp
        .send_text(phone, "Pending invite cancelled.")
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
             LEARNER — start practicing a new language\n\
             TEACHER — teach someone a new language\n\n\
             In an active conversation, just message normally.",
        )
        .await?;
    Ok(())
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
