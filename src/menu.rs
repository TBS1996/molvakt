use anyhow::Context;

use crate::conversations::{
    format_listing_menu_description, format_listing_partner, format_menu_body, handle_cancel_listing,
    handle_help, handle_list, handle_set_language, handle_set_mode, handle_switch,
    listing_is_broken, SetLanguageCommand, SetModeCommand,
};
use crate::db::{
    ConversationListing, ConversationMode, ConversationModeSetting, Db, MenuAction, MenuData,
    MenuStep, ParticipantRole,
};
use crate::onboarding::{self, start_new_conversation, start_new_exchange_conversation};
use crate::phone::{contact_label, normalize_phone, phones_match};
use crate::vocab;
use crate::whatsapp::WhatsApp;

const MAIN_LIST: &str = "menu_list";
const MAIN_SWITCH: &str = "menu_switch";
const MAIN_SET_MODE: &str = "menu_set_mode";
const MAIN_SET_LANGUAGE: &str = "menu_set_language";
const MAIN_CANCEL: &str = "menu_cancel";
const MAIN_START: &str = "menu_start";
const MAIN_HELP: &str = "menu_help";
const MAIN_REVIEW_VOCAB: &str = "menu_review_vocab";

const START_LEARNER: &str = "mode_learner";
const START_TEACHER: &str = "mode_teacher";
const START_EXCHANGE: &str = "mode_exchange";
const START_TURNS: &str = "mode_turns";

const MODE_TEACHER: &str = "menu_mode_teacher";
const MODE_LEARNER: &str = "menu_mode_learner";
const MODE_EXCHANGE: &str = "menu_mode_exchange";
const MODE_TURNS: &str = "menu_mode_turns";

const CONV_PREFIX: &str = "menu_conv_";
const MAX_MENU_CONVERSATIONS: usize = 10;

pub fn is_menu_command(text: &str) -> bool {
    matches!(text.trim().to_ascii_uppercase().as_str(), "MENU")
}

pub async fn handle_menu_command(db: &Db, whatsapp: &WhatsApp, phone: &str) -> anyhow::Result<()> {
    db.clear_menu_session(phone).await?;
    send_main_menu(db, whatsapp, phone).await
}

pub async fn handle_menu_session(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    text: &str,
    step: MenuStep,
    data: MenuData,
) -> anyhow::Result<()> {
    if is_menu_command(text) {
        return handle_menu_command(db, whatsapp, phone).await;
    }

    match step {
        MenuStep::PickConversation => {
            let action = data.action.context("missing menu action")?;
            let listings = listings_for_action(db, phone, action).await?;
            let Some(index) = parse_conversation_selection(text, listings.len()) else {
                whatsapp
                    .send_text(phone, "Invalid selection. Reply MENU to start over.")
                    .await?;
                return Ok(());
            };
            db.clear_menu_session(phone).await?;
            apply_conversation_action(db, whatsapp, phone, action, index).await
        }
        MenuStep::PickMode => {
            let Some(mode) = parse_mode_selection(text) else {
                whatsapp
                    .send_text(phone, "Invalid selection. Reply MENU to start over.")
                    .await?;
                return Ok(());
            };
            db.clear_menu_session(phone).await?;
            handle_set_mode(
                db,
                whatsapp,
                phone,
                SetModeCommand {
                    index: None,
                    mode,
                },
            )
            .await
        }
        MenuStep::AwaitLanguage => {
            db.clear_menu_session(phone).await?;
            handle_set_language(
                db,
                whatsapp,
                phone,
                SetLanguageCommand {
                    index: None,
                    language: text.trim().to_string(),
                },
            )
            .await
        }
        MenuStep::FlashcardReview => vocab::handle_flashcard_session(db, whatsapp, phone, text, data).await,
    }
}

pub async fn handle_menu_selection(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    selection: &str,
) -> anyhow::Result<bool> {
    match selection {
        MAIN_LIST => {
            handle_list(db, whatsapp, phone).await?;
            Ok(true)
        }
        MAIN_SWITCH => {
            start_conversation_pick(db, whatsapp, phone, MenuAction::Switch).await?;
            Ok(true)
        }
        MAIN_SET_MODE => {
            start_set_mode_on_active(db, whatsapp, phone).await?;
            Ok(true)
        }
        MAIN_SET_LANGUAGE => {
            start_set_language_on_active(db, whatsapp, phone).await?;
            Ok(true)
        }
        MAIN_CANCEL => {
            start_conversation_pick(db, whatsapp, phone, MenuAction::Cancel).await?;
            Ok(true)
        }
        MAIN_START => {
            send_start_menu(whatsapp, phone).await?;
            Ok(true)
        }
        MAIN_HELP => {
            handle_help(whatsapp, phone).await?;
            Ok(true)
        }
        MAIN_REVIEW_VOCAB => {
            vocab::start_review(db, whatsapp, phone).await?;
            Ok(true)
        }
        START_LEARNER => {
            db.clear_menu_session(phone).await?;
            start_new_conversation(db, whatsapp, phone, ParticipantRole::Learner).await?;
            Ok(true)
        }
        START_TEACHER => {
            db.clear_menu_session(phone).await?;
            start_new_conversation(db, whatsapp, phone, ParticipantRole::Teacher).await?;
            Ok(true)
        }
        START_EXCHANGE => {
            db.clear_menu_session(phone).await?;
            start_new_exchange_conversation(db, whatsapp, phone, ConversationMode::Exchange).await?;
            Ok(true)
        }
        START_TURNS => {
            db.clear_menu_session(phone).await?;
            start_new_exchange_conversation(db, whatsapp, phone, ConversationMode::ExchangeTurns)
                .await?;
            Ok(true)
        }
        MODE_TEACHER | MODE_LEARNER | MODE_EXCHANGE | MODE_TURNS => {
            if let Some((step, _)) = db.load_menu_session(phone).await? {
                if step == MenuStep::PickMode {
                    db.clear_menu_session(phone).await?;
                    let mode = parse_mode_selection(selection).context("invalid mode")?;
                    handle_set_mode(
                        db,
                        whatsapp,
                        phone,
                        SetModeCommand {
                            index: None,
                            mode,
                        },
                    )
                    .await?;
                    return Ok(true);
                }
            }
            Ok(false)
        }
        id if id.starts_with(CONV_PREFIX) => {
            let index = id
                .strip_prefix(CONV_PREFIX)
                .and_then(|value| value.parse().ok())
                .filter(|value| *value >= 1);
            let Some(index) = index else {
                return Ok(false);
            };
            if let Some((step, data)) = db.load_menu_session(phone).await? {
                if step == MenuStep::PickConversation {
                    let action = data.action.context("missing menu action")?;
                    db.clear_menu_session(phone).await?;
                    apply_conversation_action(db, whatsapp, phone, action, index).await?;
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

async fn send_main_menu(db: &Db, whatsapp: &WhatsApp, phone: &str) -> anyhow::Result<()> {
    let body = format_menu_body(db, phone).await?;
    whatsapp
        .send_list_menu(
            phone,
            &format!("{body}\n\nPick an action below."),
            "Open menu",
            "Actions",
            &[
                (MAIN_LIST, "View conversations", "See all your chats"),
                (MAIN_SWITCH, "Switch chat", "Change active conversation"),
                (MAIN_SET_MODE, "Set mode", "Change mode on active chat"),
                (MAIN_SET_LANGUAGE, "Set language", "Change language on active chat"),
                (MAIN_CANCEL, "Cancel invite", "Remove a pending invite"),
                (MAIN_START, "Start new", "Learner, teacher, or exchange"),
                (MAIN_REVIEW_VOCAB, "Review vocab", "Flashcards for active language"),
                (MAIN_HELP, "Help", "How molvakt works"),
            ],
        )
        .await
}

async fn send_start_menu(whatsapp: &WhatsApp, phone: &str) -> anyhow::Result<()> {
    onboarding::send_new_conversation_menu(whatsapp, phone).await
}

async fn send_mode_menu(whatsapp: &WhatsApp, phone: &str, partner: &str) -> anyhow::Result<()> {
    whatsapp
        .send_list_menu(
            phone,
            &format!("Choose a mode for your chat with {partner}."),
            "Choose mode",
            "Modes",
            &[
                (MODE_LEARNER, "Learner", "You practice, partner teaches"),
                (MODE_TEACHER, "Teacher", "You teach, partner practices"),
                (MODE_EXCHANGE, "Exchange", "Write in your learning language"),
                (MODE_TURNS, "Exchange (turns)", "Alternate languages each round"),
            ],
        )
        .await
}

async fn start_set_mode_on_active(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
) -> anyhow::Result<()> {
    let Some(listing) = require_active_listing(db, whatsapp, phone).await? else {
        return Ok(());
    };
    let partner = format_listing_partner(&listing);

    db.save_menu_session(phone, MenuStep::PickMode, &MenuData::default())
        .await?;
    send_mode_menu(whatsapp, phone, &partner).await
}

async fn start_set_language_on_active(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
) -> anyhow::Result<()> {
    let Some(listing) = require_active_listing(db, whatsapp, phone).await? else {
        return Ok(());
    };

    let partner = format_listing_partner(&listing);
    db.save_menu_session(phone, MenuStep::AwaitLanguage, &MenuData::default())
        .await?;
    whatsapp
        .send_text(
            phone,
            &format!(
                "What language should your chat with {partner} use? (e.g. Norwegian)"
            ),
        )
        .await
}

async fn require_active_listing(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
) -> anyhow::Result<Option<ConversationListing>> {
    let listings = db.list_conversations_for_phone(phone).await?;
    if listings.is_empty() {
        whatsapp
            .send_text(phone, "You don't have any conversations yet. Use Start new to create one.")
            .await?;
        return Ok(None);
    }

    let viewer_phone = normalize_phone(phone);
    let listing = if let Some(active) = listings.iter().find(|listing| listing.is_active) {
        active
    } else if listings.len() == 1 {
        &listings[0]
    } else {
        whatsapp
            .send_text(
                phone,
                "No active chat. Use Switch chat first, then try again.",
            )
            .await?;
        return Ok(None);
    };

    if listing_is_broken(listing, &viewer_phone) {
        whatsapp
            .send_text(phone, "Your active chat is invalid. Cancel it from the menu first.")
            .await?;
        return Ok(None);
    }

    if listing.is_pending {
        whatsapp
            .send_text(
                phone,
                "Your active chat is still waiting for your partner to accept.",
            )
            .await?;
        return Ok(None);
    }

    Ok(Some(listing.clone()))
}

async fn start_conversation_pick(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    action: MenuAction,
) -> anyhow::Result<()> {
    let listings = listings_for_action(db, phone, action).await?;
    if listings.is_empty() {
        let message = match action {
            MenuAction::Cancel => "You don't have any pending invites to cancel.",
            _ => "You don't have any conversations yet. Use Start new to create one.",
        };
        whatsapp.send_text(phone, message).await?;
        return Ok(());
    }

    if listings.len() == 1 {
        db.clear_menu_session(phone).await?;
        return apply_conversation_action(db, whatsapp, phone, action, 1).await;
    }

    if listings.len() <= MAX_MENU_CONVERSATIONS {
        send_conversation_menu(whatsapp, phone, &listings, action).await?;
        db.save_menu_session(
            phone,
            MenuStep::PickConversation,
            &MenuData {
                action: Some(action),
                conversation_index: None,
                ..MenuData::default()
            },
        )
        .await?;
        return Ok(());
    }

    send_conversation_text_list(whatsapp, phone, &listings, action).await?;
    db.save_menu_session(
        phone,
        MenuStep::PickConversation,
        &MenuData {
            action: Some(action),
            conversation_index: None,
            ..MenuData::default()
        },
    )
    .await
}

async fn listings_for_action(
    db: &Db,
    phone: &str,
    action: MenuAction,
) -> anyhow::Result<Vec<ConversationListing>> {
    let listings = db.list_conversations_for_phone(phone).await?;
    if action != MenuAction::Cancel {
        return Ok(listings);
    }

    let viewer_phone = normalize_phone(phone);
    Ok(listings
        .into_iter()
        .filter(|listing| {
            listing.is_pending
                || listing
                    .partner_phone
                    .as_ref()
                    .is_some_and(|partner| phones_match(partner, &viewer_phone))
        })
        .collect())
}

async fn send_conversation_menu(
    whatsapp: &WhatsApp,
    phone: &str,
    listings: &[ConversationListing],
    action: MenuAction,
) -> anyhow::Result<()> {
    let viewer_phone = normalize_phone(phone);
    let owned_rows: Vec<(String, String, String)> = listings
        .iter()
        .enumerate()
        .map(|(index, listing)| {
            let id = format!("{CONV_PREFIX}{}", index + 1);
            let (title, description) = listing_menu_labels_owned(index + 1, listing, &viewer_phone);
            (id, title, description)
        })
        .collect();

    let row_refs: Vec<(&str, &str, &str)> = owned_rows
        .iter()
        .map(|(id, title, desc)| (id.as_str(), title.as_str(), desc.as_str()))
        .collect();

    let body = match action {
        MenuAction::Switch => "Which conversation do you want to switch to?",
        MenuAction::SetMode => "Which conversation do you want to change mode on?",
        MenuAction::SetLanguage => "Which conversation do you want to set language on?",
        MenuAction::Cancel => "Which invite do you want to cancel?",
    };

    whatsapp
        .send_list_menu(phone, body, "Choose chat", "Conversations", &row_refs)
        .await
}

async fn send_conversation_text_list(
    whatsapp: &WhatsApp,
    phone: &str,
    listings: &[ConversationListing],
    action: MenuAction,
) -> anyhow::Result<()> {
    let viewer_phone = normalize_phone(phone);
    let mut lines = vec!["Your conversations:".to_string()];
    for (index, listing) in listings.iter().enumerate() {
        lines.push(crate::conversations::format_listing_line(
            index + 1,
            listing,
            &viewer_phone,
        ));
    }
    lines.push(String::new());
    lines.push("Reply with the number of your choice.".into());
    lines.push("Reply MENU to start over.".into());

    let intro = match action {
        MenuAction::Switch => "You have more than 10 conversations. ",
        MenuAction::SetMode => "You have more than 10 conversations. ",
        MenuAction::SetLanguage => "You have more than 10 conversations. ",
        MenuAction::Cancel => "You have more than 10 pending invites. ",
    };
    lines.insert(0, intro.to_string());

    whatsapp.send_text(phone, &lines.join("\n")).await
}

fn listing_menu_labels_owned(
    index: usize,
    listing: &ConversationListing,
    viewer_phone: &str,
) -> (String, String) {
    let partner = listing
        .partner_phone
        .as_deref()
        .map(|phone| contact_label(phone, listing.partner_display_name.as_deref()))
        .unwrap_or_else(|| "unknown".into());
    let mut title = format!("{index}. {partner}");
    if listing.is_active {
        title.push('*');
    }
    if title.len() > 24 {
        title.truncate(24);
    }

    let description = format_listing_menu_description(listing, viewer_phone);
    (title, description)
}

async fn apply_conversation_action(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    action: MenuAction,
    index: usize,
) -> anyhow::Result<()> {
    match action {
        MenuAction::Switch => handle_switch(db, whatsapp, phone, index).await,
        MenuAction::Cancel => {
            let listings = listings_for_action(db, phone, MenuAction::Cancel).await?;
            let Some(listing) = listings.get(index.saturating_sub(1)) else {
                whatsapp
                    .send_text(phone, "Invalid selection. Reply MENU to start over.")
                    .await?;
                return Ok(());
            };
            handle_cancel_listing(db, whatsapp, phone, listing).await
        }
        MenuAction::SetMode | MenuAction::SetLanguage => {
            whatsapp
                .send_text(phone, "Reply MENU to start over.")
                .await?;
            Ok(())
        }
    }
}

fn parse_conversation_selection(text: &str, count: usize) -> Option<usize> {
    if let Some(id) = text.strip_prefix(CONV_PREFIX) {
        return id.parse().ok().filter(|index| *index >= 1 && *index <= count);
    }
    text.trim()
        .parse::<usize>()
        .ok()
        .filter(|index| *index >= 1 && *index <= count)
}

fn parse_mode_selection(text: &str) -> Option<ConversationModeSetting> {
    match text.trim() {
        MODE_TEACHER | "TEACHER" => Some(ConversationModeSetting::Teacher),
        MODE_LEARNER | "LEARNER" => Some(ConversationModeSetting::Learner),
        MODE_EXCHANGE | "EXCHANGE" => Some(ConversationModeSetting::Exchange),
        MODE_TURNS | "EXCHANGE-TURNS" | "EXCHANGE_TURNS" | "TURNS" => {
            Some(ConversationModeSetting::ExchangeTurns)
        }
        _ => None,
    }
}
