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
    pub interactive: Option<InteractiveBody>,
}

#[derive(Debug, Deserialize)]
pub struct InteractiveBody {
    #[serde(rename = "type")]
    pub interactive_type: String,
    pub button_reply: Option<InteractiveReply>,
    pub list_reply: Option<InteractiveReply>,
}

#[derive(Debug, Deserialize)]
pub struct InteractiveReply {
    pub id: String,
    #[allow(dead_code)]
    pub title: String,
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

    pub fn parse_incoming_messages(body: &str) -> anyhow::Result<Vec<(String, String)>> {
        let payload: WebhookPayload = serde_json::from_str(body).context("invalid webhook json")?;
        let mut messages = Vec::new();

        for entry in payload.entry.unwrap_or_default() {
            for change in entry.changes.unwrap_or_default() {
                for message in change.value.and_then(|v| v.messages).unwrap_or_default() {
                    if let Some(text) = message_body(&message) {
                        messages.push((message.from, text));
                    }
                }
            }
        }

        Ok(messages)
    }

    pub async fn send_review_choice_list(
    &self,
    to: &str,
    body: &str,
    _button_label: &str,
    rows: &[(&str, &str, &str)],
) -> anyhow::Result<()> {
    let buttons: Vec<_> = rows
        .iter()
        .map(|(id, title, _description)| {
            serde_json::json!({
                "type": "reply",
                "reply": {
                    "id": id,
                    "title": title,
                }
            })
        })
        .collect();

    self.send_interactive(
        to,
        serde_json::json!({
            "type": "interactive",
            "interactive": {
                "type": "button",
                "body": {
                    "text": body
                },
                "action": {
                    "buttons": buttons
                }
            }
        }),
    )
    .await
}

    pub async fn send_text(&self, to: &str, body: &str) -> anyhow::Result<()> {
        self.send_interactive(
            to,
            serde_json::json!({
                "type": "text",
                "text": { "body": body },
            }),
        )
        .await
    }

    async fn send_interactive(&self, to: &str, content: serde_json::Value) -> anyhow::Result<()> {
        let url = format!(
            "https://graph.facebook.com/v21.0/{}/messages",
            self.phone_number_id
        );

        let mut payload = serde_json::json!({
            "messaging_product": "whatsapp",
            "to": to,
        });
        payload
            .as_object_mut()
            .context("invalid message payload")?
            .extend(content.as_object().cloned().context("invalid message content")?);

        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .json(&payload)
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

fn message_body(message: &IncomingMessage) -> Option<String> {
    match message.message_type.as_str() {
        "text" => message.text.as_ref().map(|text| text.body.clone()),
        "interactive" => {
            let interactive = message.interactive.as_ref()?;
            match interactive.interactive_type.as_str() {
                "button_reply" => interactive.button_reply.as_ref().map(|reply| reply.id.clone()),
                "list_reply" => interactive.list_reply.as_ref().map(|reply| reply.id.clone()),
                _ => None,
            }
        }
        _ => None,
    }
}
