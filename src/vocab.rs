use crate::conversations::listing_is_broken;
use crate::db::{Db, MenuData, MenuStep, VocabCard};
use crate::llm::Llm;
use crate::phone::{contact_label, normalize_phone};
use crate::whatsapp::WhatsApp;

pub const VOCAB_SHOW: &str = "vocab_show";
pub const VOCAB_GOOD: &str = "vocab_good";
pub const VOCAB_AGAIN: &str = "vocab_again";

pub fn is_vocab_button(text: &str) -> bool {
    matches!(text, VOCAB_SHOW | VOCAB_GOOD | VOCAB_AGAIN)
}

pub async fn extract_from_message(
    db: &Db,
    user_phone: &str,
    message: &str,
    learning_language: &str,
    partner_phone: &str,
    conversation_id: i64,
) -> anyhow::Result<()> {
    let llm = Llm::for_vocabulary(learning_language)?;
    let items = llm.extract_vocabulary(message).await?;
    if items.is_empty() {
        return Ok(());
    }

    for (term, translation) in items {
        db.insert_vocab_card(
            user_phone,
            learning_language,
            &term,
            &translation,
            Some(partner_phone),
            Some(conversation_id),
        )
        .await?;
        db.insert_vocab_card(
            user_phone,
            learning_language,
            &translation,
            &term,
            Some(partner_phone),
            Some(conversation_id),
        )
        .await?;
    }

    Ok(())
}

pub fn spawn_extract_from_message(
    db: Db,
    user_phone: String,
    message: String,
    learning_language: String,
    partner_phone: String,
    conversation_id: i64,
) {
    tokio::spawn(async move {
        if let Err(error) = extract_from_message(
            &db,
            &user_phone,
            &message,
            &learning_language,
            &partner_phone,
            conversation_id,
        )
        .await
        {
            eprintln!(
                "vocab extraction failed for {}: {error:?}",
                user_phone
            );
        }
    });
}

pub async fn start_review(db: &Db, whatsapp: &WhatsApp, phone: &str) -> anyhow::Result<()> {
    let Some((language, _partner)) = review_language_for_phone(db, phone).await? else {
        whatsapp
            .send_text(
                phone,
                "No active chat to review vocab for. Switch to a conversation first, or start one.",
            )
            .await?;
        return Ok(());
    };

    let total = db.count_vocab_cards(phone, &language).await?;
    if total == 0 {
        whatsapp
            .send_text(
                phone,
                &format!(
                    "No vocab cards for {language} yet — they'll appear as you chat."
                ),
            )
            .await?;
        return Ok(());
    }

    let due = db.count_due_vocab_cards(phone, &language).await?;
    if due == 0 {
        whatsapp
            .send_text(
                phone,
                &format!(
                    "No {language} cards due right now. You have {total} card(s) — check back later."
                ),
            )
            .await?;
        return Ok(());
    }

    show_next_card(db, whatsapp, phone, &language, None).await
}

pub async fn handle_vocab_button(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    button: &str,
    card_id: i64,
) -> anyhow::Result<()> {
    let card = db
        .get_vocab_card(card_id)
        .await?
        .filter(|card| normalize_phone(&card.user_phone) == normalize_phone(phone));

    let Some(card) = card else {
        whatsapp
            .send_text(phone, "That card is no longer available. Reply MENU to start over.")
            .await?;
        db.clear_menu_session(phone).await?;
        return Ok(());
    };

    match button {
        VOCAB_SHOW => show_card_answer(whatsapp, phone, &card).await,
        VOCAB_GOOD => {
            db.review_vocab_card_pass(card_id).await?;
            finish_or_next(db, whatsapp, phone, &card, true).await
        }
        VOCAB_AGAIN => {
            db.review_vocab_card_fail(card_id).await?;
            finish_or_next(db, whatsapp, phone, &card, false).await
        }
        _ => Ok(()),
    }
}

pub async fn handle_flashcard_session(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    text: &str,
    data: MenuData,
) -> anyhow::Result<bool> {
    if is_vocab_button(text) {
        let Some(card_id) = data.flashcard_id else {
            whatsapp
                .send_text(phone, "Session expired. Reply MENU to start over.")
                .await?;
            db.clear_menu_session(phone).await?;
            return Ok(true);
        };
        handle_vocab_button(db, whatsapp, phone, text, card_id).await?;
        return Ok(true);
    }

    Ok(false)
}

async fn review_language_for_phone(
    db: &Db,
    phone: &str,
) -> anyhow::Result<Option<(String, Option<String>)>> {
    let listings = db.list_conversations_for_phone(phone).await?;
    let viewer_phone = normalize_phone(phone);

    let listing = listings
        .iter()
        .find(|listing| listing.is_active)
        .or_else(|| listings.iter().find(|listing| !listing.is_pending))
        .or_else(|| listings.first());

    let Some(listing) = listing else {
        return Ok(None);
    };

    if listing_is_broken(listing, &viewer_phone) || listing.is_pending {
        return Ok(None);
    }

    let language = if listing.mode.is_exchange() {
        listing
            .learning_language
            .clone()
            .unwrap_or_else(|| listing.target_language.clone())
    } else if listing.role == crate::db::ParticipantRole::Learner {
        listing.target_language.clone()
    } else {
        return Ok(None);
    };

    Ok(Some((language, listing.partner_phone.clone())))
}

async fn show_next_card(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    language: &str,
    after: Option<&VocabCard>,
) -> anyhow::Result<()> {
    let skip_inverse = after.map(|card| (card.front.as_str(), card.back.as_str()));
    let Some(card) = db
        .next_due_vocab_card(phone, language, skip_inverse)
        .await?
    else {
        whatsapp
            .send_text(phone, "All done for now — no more cards due.")
            .await?;
        db.clear_menu_session(phone).await?;
        return Ok(());
    };

    db.save_menu_session(
        phone,
        MenuStep::FlashcardReview,
        &MenuData {
            flashcard_id: Some(card.id),
            ..MenuData::default()
        },
    )
    .await?;

    send_card_prompt(whatsapp, phone, &card).await
}

async fn send_card_prompt(whatsapp: &WhatsApp, phone: &str, card: &VocabCard) -> anyhow::Result<()> {
    let partner_note = card
        .partner_phone
        .as_deref()
        .map(|partner| format!("\n(from chat with {})", contact_label(partner, None)))
        .unwrap_or_default();

    let body = format!(
        "Flashcard ({language}){partner_note}\n\n{front}",
        language = card.language,
        front = card.front,
    );

    whatsapp
        .send_review_choice_list(phone, &body, "Show answer", &[(VOCAB_SHOW, "Show answer", "")])
        .await
}

async fn show_card_answer(whatsapp: &WhatsApp, phone: &str, card: &VocabCard) -> anyhow::Result<()> {
    let body = format!(
        "Flashcard ({language})\n\n{front}\n\n→ {back}",
        language = card.language,
        front = card.front,
        back = card.back,
    );

    whatsapp
        .send_review_choice_list(
            phone,
            &body,
            "Rate",
            &[
                (VOCAB_GOOD, "Got it", ""),
                (VOCAB_AGAIN, "Again", ""),
            ],
        )
        .await
}

async fn finish_or_next(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
    after: &VocabCard,
    passed: bool,
) -> anyhow::Result<()> {
    let language = &after.language;
    let remaining = db.count_due_vocab_cards(phone, language).await?;
    if remaining == 0 {
        let message = if passed {
            "Nice — that was the last card due for now."
        } else {
            "That card will come back soon. No more due right now."
        };
        whatsapp.send_text(phone, message).await?;
        db.clear_menu_session(phone).await?;
        return Ok(());
    }

    show_next_card(db, whatsapp, phone, language, Some(after)).await
}
