use std::str::FromStr;

use chrono::{DateTime, NaiveDateTime, Timelike, Utc};
use chrono_tz::Tz;

use crate::conversations::{format_listing_partner, format_listing_role_desc, listing_awaits_user_reply};
use crate::db::{ConversationListing, Db};
use crate::phone::normalize_phone;
use crate::whatsapp::WhatsApp;

const DEFAULT_TIMEZONE: &str = "Europe/Oslo";
const MORNING_HOUR: u32 = 8;
pub const DISABLE_DAILY_REMINDERS_BUTTON: &str = "reminders_off";

pub fn is_disable_daily_reminders_button(text: &str) -> bool {
    text == DISABLE_DAILY_REMINDERS_BUTTON
}

pub async fn run_tick(db: &Db, whatsapp: &WhatsApp) -> anyhow::Result<()> {
    let phones = db.list_registered_user_phones().await?;
    for phone in phones {
        if let Err(error) = maybe_send_morning_reminder(db, whatsapp, &phone).await {
            eprintln!("morning reminder for {phone}: {error:?}");
        }
    }
    Ok(())
}

async fn maybe_send_morning_reminder(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
) -> anyhow::Result<()> {
    let settings = db.get_user_reminder_settings(phone).await?;
    if !settings.morning_reminders_enabled {
        return Ok(());
    }

    let timezone = settings
        .timezone
        .as_deref()
        .unwrap_or(DEFAULT_TIMEZONE);
    let tz = Tz::from_str(timezone).unwrap_or_else(|_| {
        Tz::from_str(DEFAULT_TIMEZONE).expect("Europe/Oslo is a valid timezone")
    });

    let local_now = Utc::now().with_timezone(&tz);
    if local_now.hour() != MORNING_HOUR {
        return Ok(());
    }

    let local_date = local_now.format("%Y-%m-%d").to_string();
    if settings.last_morning_reminder_date.as_deref() == Some(local_date.as_str()) {
        return Ok(());
    }

    if !user_inactive_for_hour(settings.last_message_at.as_deref()) {
        return Ok(());
    }

    let viewer_phone = normalize_phone(phone);
    let listings = db.list_conversations_for_phone(phone).await?;
    let pending: Vec<&ConversationListing> = listings
        .iter()
        .filter(|listing| listing_awaits_user_reply(listing, &viewer_phone))
        .collect();
    if pending.is_empty() {
        return Ok(());
    }

    let body = format_reminder_message(&pending);
    whatsapp
        .send_review_choice_list(
            phone,
            &body,
            "Options",
            &[(
                DISABLE_DAILY_REMINDERS_BUTTON,
                "Turn off reminders",
                "",
            )],
        )
        .await?;
    db.mark_morning_reminder_sent(phone, &local_date).await?;
    Ok(())
}

fn user_inactive_for_hour(last_message_at: Option<&str>) -> bool {
    let Some(raw) = last_message_at else {
        return true;
    };
    let Ok(naive) = NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S") else {
        return true;
    };
    let at = DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc);
    Utc::now().signed_duration_since(at) >= chrono::Duration::hours(1)
}

fn format_reminder_message(listings: &[&ConversationListing]) -> String {
    let mut lines = vec![
        "Good morning! You have chats waiting for a reply:".to_string(),
        String::new(),
    ];

    for listing in listings {
        let partner = format_listing_partner(listing);
        let role_desc = format_listing_role_desc(listing);
        let status = crate::conversations::format_listing_status_text(listing);
        let detail = if status.is_empty() {
            role_desc
        } else {
            format!("{role_desc}, {status}")
        };
        lines.push(format!("• {partner} — {detail}"));
    }

    lines.push(String::new());
    lines.push("Open WhatsApp and reply when you're ready, or send MENU for options.".into());
    lines.join("\n")
}

pub fn parse_timezone_name(text: &str) -> Option<String> {
    let rest = text.trim().strip_prefix("SET TIMEZONE ")?;
    let timezone = rest.trim();
    if timezone.is_empty() {
        return None;
    }
    Tz::from_str(timezone).ok()?;
    Some(timezone.to_string())
}

pub async fn handle_set_timezone(db: &Db, whatsapp: &WhatsApp, phone: &str, timezone: &str) -> anyhow::Result<()> {
    db.set_user_timezone(phone, timezone).await?;
    whatsapp
        .send_text(phone, &format!("Timezone set to {timezone}. Morning reminders arrive around 8:00 local time."))
        .await?;
    Ok(())
}

pub async fn handle_disable_daily_reminders(
    db: &Db,
    whatsapp: &WhatsApp,
    phone: &str,
) -> anyhow::Result<()> {
    db.set_morning_reminders_enabled(phone, false).await?;
    whatsapp
        .send_text(phone, "Daily reminders turned off.")
        .await?;
    Ok(())
}
