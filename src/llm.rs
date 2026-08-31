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

const EXPLANATIONS_IN_ENGLISH: &str =
    "All feedback, grammar help, and explanations must be written in English only — \
     never in the student's native language or the target language. \
     You may still quote words or short phrases in the target language when teaching vocabulary.";

pub struct Llm {
    client: Client<OpenAIConfig>,
    model: String,
    target_language: String,
    source_language: String,
}

#[derive(Deserialize)]
pub struct JudgmentResponse {
    pub accepted: bool,
    #[serde(default)]
    pub feedback: String,
}

#[derive(Deserialize)]
struct VocabExtractionResponse {
    #[serde(default)]
    items: Vec<VocabItem>,
}

#[derive(Deserialize)]
struct VocabItem {
    term: String,
    translation: String,
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

    pub fn for_exchange(
        learning_language: &str,
        partner_learning_language: &str,
    ) -> Result<Self> {
        let api_key =
            std::env::var("OPENAI_API_KEY").context("OPENAI_API_KEY must be set")?;
        let model = std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());

        Ok(Self {
            client: Client::with_config(OpenAIConfig::new().with_api_key(api_key)),
            model,
            target_language: learning_language.to_string(),
            source_language: partner_learning_language.to_string(),
        })
    }

    pub fn for_vocabulary(learning_language: &str) -> Result<Self> {
        Self::for_exchange(learning_language, "English")
    }

    pub fn source_language(&self) -> &str {
        &self.source_language
    }

    pub fn target_language(&self) -> &str {
        &self.target_language
    }

    pub async fn validate_teacher_message(&self, message: &str) -> Result<JudgmentResponse> {
        let system = format!(
            "You check whether a native speaker's message is written in {target}. \
             Accept any natural {target} message — including informal, casual, elliptical, \
             or abbreviated phrasing (e.g. implied words, short follow-up questions). \
             Do NOT reject for style, formality, clarity, or how you would phrase it differently. \
             Ignore spelling mistakes and missing diacritics or special letters \
             (e.g. Turkish 'nasilsin' for 'nasılsın', or Norwegian without æ/ø/å) \
             as long as the meaning is clear. \
             Reject ONLY if the message is primarily in another language (e.g. {source}) \
             or is not real text in any language. A few loanwords are fine. \
             When in doubt, accept. {english_rule} \
             Respond with JSON only: \
             {{\"accepted\": true/false, \"feedback\": \"brief explanation if rejected\"}}",
            target = self.target_language,
            source = self.source_language,
            english_rule = EXPLANATIONS_IN_ENGLISH,
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
             if the meaning is correct. Ignore spelling mistakes and missing diacritics. \
             {english_rule} \
             Respond with JSON only: \
             {{\"accepted\": true/false, \"feedback\": \"short explanation\"}}",
            target = self.target_language,
            source = self.source_language,
            english_rule = EXPLANATIONS_IN_ENGLISH,
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
            "You are a patient language tutor teaching {target} to a {source} speaker. \
             The student did not understand this message and needs a clear lesson. \
             {format}",
            target = self.target_language,
            source = self.source_language,
            format = teaching_format(&self.target_language),
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
            "You are a patient language tutor teaching {target} to a {source} speaker. \
             The student tried to translate a message but got it wrong. \
             Give a full lesson using the format below, and add a short note on what was \
             off in their attempt and what to look for next time. \
             {format}",
            target = self.target_language,
            source = self.source_language,
            format = teaching_format(&self.target_language),
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
             Accept any reply that a native speaker would understand in casual conversation — \
             including informal, elliptical, or abbreviated phrasing (e.g. short follow-ups like \
             'og du?'). Do NOT reject for style, formality, spelling mistakes, missing diacritics \
             or special letters (e.g. 'nasilsin' for 'nasılsın'), or because you would phrase it \
             differently. Reject only for clear grammatical errors or meaning that would confuse \
             a native speaker, or if the reply is in the wrong language. \
             If they wrote in {source} instead of {target}, reject but help them: briefly \
             explain what they were trying to say, teach the grammar they need, and give \
             useful words or patterns for answering in {target} — without writing the full \
             reply for them. For other rejections, give a short hint without rewriting \
             the sentence. When in doubt, accept. {english_rule} \
             Respond with JSON only: \
             {{\"accepted\": true/false, \"feedback\": \"hints, mini-lesson, or brief praise\"}}",
            target = self.target_language,
            source = self.source_language,
            english_rule = EXPLANATIONS_IN_ENGLISH,
        );

        let user = format!(
            "Conversation so far:\n{history}\n\n\
             Student's draft reply in {target}: {attempt}",
            history = format_history(history),
            target = self.target_language,
        );

        self.judge(&system, &user).await
    }

    pub async fn extract_vocabulary(&self, message: &str) -> Result<Vec<(String, String)>> {
        let system = format!(
            "You extract vocabulary for flashcards from a {target} message. \
             The student is learning {target}. \
             Pick useful individual words and short phrases (up to ~5 words) — \
             common collocations, idioms, or expressions worth memorizing. \
             Do NOT include the full sentence unless it is a well-known fixed phrase \
             (e.g. a greeting or proverb). Skip names, filler, and words that are \
             obvious cognates with no learning value. \
             Return JSON only: {{\"items\": [{{\"term\": \"...\", \"translation\": \"...\"}}]}}. \
             Each term must be in {target}. Each translation must be in English only — \
             never translate into any other language. \
             If nothing is worth adding, return {{\"items\": []}}.",
            target = self.target_language,
        );

        let user = format!("Message in {target}:\n{message}", target = self.target_language);

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

        let parsed: VocabExtractionResponse =
            serde_json::from_str(&content).context("failed to parse vocabulary JSON")?;

        Ok(parsed
            .items
            .into_iter()
            .filter_map(|item| {
                let term = item.term.trim();
                let translation = item.translation.trim();
                if term.is_empty() || translation.is_empty() {
                    None
                } else {
                    Some((term.to_string(), translation.to_string()))
                }
            })
            .collect())
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

fn teaching_format(target: &str) -> String {
    format!(
        "Structure your answer for WhatsApp (plain text, short labeled sections). \
         {english_rule}\n\
         Translation — natural English equivalent.\n\
         Meaning — one or two sentences in English on what the message is saying.\n\
         Grammar — explain in English: sentence structure, word order, verb forms, articles, cases, \
         or other patterns a beginner should learn from this message.\n\
         Words — break down each word or short phrase: the form as written, its English meaning, \
         and its role in this sentence (explained in English).\n\
         How to reply — useful patterns or phrases for answering a message like this \
         in {target}; show building blocks in {target} with brief English notes on usage, \
         not a full scripted answer.\n\
         Be thorough and beginner-friendly. Prefer teaching over brevity.",
        english_rule = EXPLANATIONS_IN_ENGLISH,
        target = target,
    )
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
            HistoryEntry::Exchange { sender, content } => format!("{sender}: {content}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}
