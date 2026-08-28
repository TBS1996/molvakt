use anyhow::{Context, Result};
use async_openai::config::OpenAIConfig;
use async_openai::types::chat::{
    ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
    CreateChatCompletionRequestArgs, ResponseFormat,
};
use async_openai::Client;

use serde::Deserialize;

use crate::db::Conversation;
use crate::history::HistoryEntry;

pub struct Llm {
    client: Client<OpenAIConfig>,
    model: String,
    target_language: String,
    source_language: String,
}

#[derive(Deserialize)]
pub struct JudgmentResponse {
    pub accepted: bool,
    pub feedback: String,
}

impl Llm {
    pub fn from_env(conversation: &Conversation) -> Result<Self> {
        let api_key =
            std::env::var("OPENAI_API_KEY").context("OPENAI_API_KEY must be set")?;
        let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());

        Ok(Self {
            client: Client::with_config(OpenAIConfig::new().with_api_key(api_key)),
            model,
            target_language: conversation.target_language.clone(),
            source_language: conversation.source_language.clone(),
        })
    }

    pub fn source_language(&self) -> &str {
        &self.source_language
    }

    pub fn target_language(&self) -> &str {
        &self.target_language
    }

    pub async fn validate_teacher_message(&self, message: &str) -> Result<JudgmentResponse> {
        let system = format!(
            "You check whether a message is written in {target}. \
             Reject if it is primarily in another language (e.g. {source}). \
             A few loanwords are fine if the sentence is clearly {target}. \
             Respond with JSON only: \
             {{\"accepted\": true/false, \"feedback\": \"brief explanation if rejected\"}}",
            target = self.target_language,
            source = self.source_language,
        );

        let user = format!("Message: {message}");

        self.judge(&system, &user).await
    }

    pub async fn rate_review_attempt(
        &self,
        attempt: &str,
        original: &str,
        history: &[HistoryEntry],
    ) -> Result<JudgmentResponse> {
        let system = format!(
            "You are a language tutor helping a student learning {target}. \
             Judge whether their translation of a message from {target} into {source} \
             captures the meaning well enough. Be fair: minor wording differences are fine \
             if the meaning is correct. Respond with JSON only: \
             {{\"accepted\": true/false, \"feedback\": \"short explanation\"}}",
            target = self.target_language,
            source = self.source_language,
        );

        let user = format!(
            "Conversation so far:\n{history}\n\n\
             Message in {target}: {original}\n\
             Student's translation into {source}: {attempt}",
            history = format_history(history),
            target = self.target_language,
            source = self.source_language,
        );

        self.judge(&system, &user).await
    }

    pub async fn teach_message(&self, original: &str, history: &[HistoryEntry]) -> Result<String> {
        let system = format!(
            "You are a language tutor teaching {target} to a {source} speaker. \
             Explain the message clearly and concisely. Include:\n\
             1. A natural translation into {source}\n\
             2. Brief grammar notes for the sentence structure\n\
             3. Key words with meaning in isolation and in this sentence's context",
            target = self.target_language,
            source = self.source_language,
        );

        let user = format!(
            "Conversation so far:\n{history}\n\n\
             Explain this {target} message:\n{original}",
            history = format_history(history),
            target = self.target_language,
        );

        self.complete(&system, &user).await
    }

    pub async fn teach_message_with_tips(
        &self,
        original: &str,
        wrong_attempt: &str,
        history: &[HistoryEntry],
    ) -> Result<String> {
        let system = format!(
            "You are a language tutor teaching {target} to a {source} speaker. \
             The student tried to translate a message but got it wrong. \
             Give the full explanation (translation, grammar, word breakdown) and \
             specific tips based on what they got wrong in their attempt.",
            target = self.target_language,
            source = self.source_language,
        );

        let user = format!(
            "Conversation so far:\n{history}\n\n\
             Message in {target}: {original}\n\
             Student's incorrect translation: {wrong_attempt}",
            history = format_history(history),
            target = self.target_language,
        );

        self.complete(&system, &user).await
    }

    pub async fn validate_reply(
        &self,
        attempt: &str,
        history: &[HistoryEntry],
    ) -> Result<JudgmentResponse> {
        let system = format!(
            "You are a language tutor helping a student write in {target}. \
             Check whether their reply is grammatically correct {target} and appropriate \
             for the conversation. If there are errors, give helpful hints without giving \
             away the full corrected sentence. Accept if it's good enough for a learner \
             at conversational level. Respond with JSON only: \
             {{\"accepted\": true/false, \"feedback\": \"hints or brief praise\"}}",
            target = self.target_language,
        );

        let user = format!(
            "Conversation so far:\n{history}\n\n\
             Student's draft reply in {target}: {attempt}",
            history = format_history(history),
            target = self.target_language,
        );

        self.judge(&system, &user).await
    }

    async fn complete(&self, system: &str, user: &str) -> Result<String> {
        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model)
            .messages([
                ChatCompletionRequestSystemMessageArgs::default()
                    .content(system)
                    .build()?
                    .into(),
                ChatCompletionRequestUserMessageArgs::default()
                    .content(user)
                    .build()?
                    .into(),
            ])
            .build()?;

        let response = self.client.chat().create(request).await?;
        response
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .context("empty response from model")
    }

    async fn judge(&self, system: &str, user: &str) -> Result<JudgmentResponse> {
        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model)
            .messages([
                ChatCompletionRequestSystemMessageArgs::default()
                    .content(system)
                    .build()?
                    .into(),
                ChatCompletionRequestUserMessageArgs::default()
                    .content(user)
                    .build()?
                    .into(),
            ])
            .response_format(ResponseFormat::JsonObject)
            .build()?;

        let response = self.client.chat().create(request).await?;
        let content = response
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .context("empty response from model")?;

        serde_json::from_str(&content).context("failed to parse model JSON response")
    }
}

pub fn format_history(history: &[HistoryEntry]) -> String {
    if history.is_empty() {
        return "(no prior messages)".into();
    }

    history
        .iter()
        .map(|entry| match entry {
            HistoryEntry::Teacher(message) => format!("Teacher: {message}"),
            HistoryEntry::Learner(message) => format!("Learner: {message}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}
