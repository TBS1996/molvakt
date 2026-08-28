use anyhow::Context;
use serde::Deserialize;

#[derive(Clone)]
pub struct WhatsApp {
    http: reqwest::Client,
    token: String,
    phone_number_id: String,
}

#[derive(Debug, Deserialize)]
pub struct WebhookPayload {
    pub entry: Option<Vec<WebhookEntry>>,
}

#[derive(Debug, Deserialize)]
pub struct WebhookEntry {
    pub changes: Option<Vec<WebhookChange>>,
}

#[derive(Debug, Deserialize)]
pub struct WebhookChange {
    pub value: Option<WebhookValue>,
}

#[derive(Debug, Deserialize)]
pub struct WebhookValue {
    pub messages: Option<Vec<IncomingMessage>>,
}

#[derive(Debug, Deserialize)]
pub struct IncomingMessage {
    pub from: String,
    #[serde(rename = "type")]
    pub message_type: String,
    pub text: Option<TextBody>,
}

#[derive(Debug, Deserialize)]
pub struct TextBody {
    pub body: String,
}

impl WhatsApp {
    pub fn from_env() -> anyhow::Result<Self> {
        let token = std::env::var("WHATSAPP_TOKEN")
            .or_else(|_| std::env::var("WHATSAPP_ACCESS_TOKEN"))
            .context("WHATSAPP_TOKEN must be set")?;
        let phone_number_id = std::env::var("WHATSAPP_PHONE_NUMBER_ID")
            .context("WHATSAPP_PHONE_NUMBER_ID must be set")?;

        Ok(Self {
            http: reqwest::Client::new(),
            token,
            phone_number_id,
        })
    }

    pub fn parse_text_messages(body: &str) -> anyhow::Result<Vec<(String, String)>> {
        let payload: WebhookPayload = serde_json::from_str(body).context("invalid webhook json")?;
        let mut messages = Vec::new();

        for entry in payload.entry.unwrap_or_default() {
            for change in entry.changes.unwrap_or_default() {
                for message in change.value.and_then(|v| v.messages).unwrap_or_default() {
                    if message.message_type == "text" {
                        if let Some(text) = message.text {
                            messages.push((message.from, text.body));
                        }
                    }
                }
            }
        }

        Ok(messages)
    }

    pub async fn send_text(&self, to: &str, body: &str) -> anyhow::Result<()> {
        let url = format!(
            "https://graph.facebook.com/v21.0/{}/messages",
            self.phone_number_id
        );

        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .json(&serde_json::json!({
                "messaging_product": "whatsapp",
                "to": to,
                "type": "text",
                "text": { "body": body },
            }))
            .send()
            .await
            .context("failed to send whatsapp message")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("whatsapp api error {status}: {body}");
        }

        Ok(())
    }
}
