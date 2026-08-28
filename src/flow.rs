use serde::{Deserialize, Serialize};

use crate::history::HistoryEntry;
use crate::llm::Llm;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum LearnerSession {
    #[default]
    Idle,
    Reviewing(ReviewState),
    Replying(ReplyState),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewState {
    pub message: String,
    pub phase: ReviewPhase,
    pub attempts: Vec<ReviewAttempt>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReviewPhase {
    Choosing,
    Teaching,
    Quizzing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewAttempt {
    pub message: String,
    pub accepted: bool,
    pub feedback: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplyState {
    pub teacher_message: String,
    pub understanding: UnderstandingSummary,
    pub attempts: Vec<ReplyAttempt>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum UnderstandingSummary {
    UnderstoodCompletely,
    DidNotUnderstand,
    TranslatedCorrectly { translation_attempts: usize },
    TranslatedIncorrectly { translation_attempts: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplyAttempt {
    pub message: String,
    pub accepted: bool,
    pub feedback: String,
}

pub struct LearnerTurn {
    pub learner_messages: Vec<String>,
    pub teacher_message: Option<String>,
    pub completed_reply: Option<String>,
}

impl ReviewState {
    pub fn new(message: String) -> Self {
        Self {
            message,
            phase: ReviewPhase::Choosing,
            attempts: Vec::new(),
        }
    }
}

impl ReplyState {
    pub fn new(teacher_message: String, understanding: UnderstandingSummary) -> Self {
        Self {
            teacher_message,
            understanding,
            attempts: Vec::new(),
        }
    }
}

pub fn begin_review(teacher_message: String) -> (LearnerSession, Vec<String>) {
    let session = LearnerSession::Reviewing(ReviewState::new(teacher_message.clone()));
    let messages = vec![
        format!("New message from your teacher:\n\"{teacher_message}\""),
        review_choice_prompt(),
    ];
    (session, messages)
}

pub fn review_choice_prompt() -> String {
    "How well did you understand this message?\n\
     1 — I understand completely\n\
     2 — I don't understand\n\
     3 — I might understand"
        .to_string()
}

pub async fn handle_learner_message(
    session: &mut LearnerSession,
    input: &str,
    history: &[HistoryEntry],
    llm: &Llm,
) -> anyhow::Result<LearnerTurn> {
    let mut turn = LearnerTurn {
        learner_messages: Vec::new(),
        teacher_message: None,
        completed_reply: None,
    };

    let input = input.trim();
    if input.is_empty() {
        turn.learner_messages
            .push("Please send a text message.".into());
        return Ok(turn);
    }

    loop {
        match session {
            LearnerSession::Idle => {
                turn.learner_messages
                    .push("Waiting for a new message from your teacher.".into());
                break;
            }
            LearnerSession::Reviewing(review) => match review.phase {
                ReviewPhase::Choosing => match input {
                    "1" => {
                        *session = LearnerSession::Replying(ReplyState::new(
                            review.message.clone(),
                            UnderstandingSummary::UnderstoodCompletely,
                        ));
                        turn.learner_messages.push(format!(
                            "Write your reply in {}:",
                            llm.target_language()
                        ));
                        break;
                    }
                    "2" => {
                        review.phase = ReviewPhase::Teaching;
                        continue;
                    }
                    "3" => {
                        review.phase = ReviewPhase::Quizzing;
                        turn.learner_messages.push(format!(
                            "Try to translate the message into {}.",
                            llm.source_language()
                        ));
                        break;
                    }
                    _ => {
                        turn.learner_messages.push(format!(
                            "Invalid choice. Pick 1, 2, or 3.\n\n{}",
                            review_choice_prompt()
                        ));
                        break;
                    }
                },
                ReviewPhase::Teaching => {
                    let teaching = llm.teach_message(&review.message, history).await?;
                    *session = LearnerSession::Replying(ReplyState::new(
                        review.message.clone(),
                        UnderstandingSummary::DidNotUnderstand,
                    ));
                    turn.learner_messages.push(teaching);
                    turn.learner_messages.push(format!(
                        "Write your reply in {}:",
                        llm.target_language()
                    ));
                    break;
                }
                ReviewPhase::Quizzing => {
                    let judgment = llm
                        .rate_review_attempt(input, &review.message, history)
                        .await?;
                    review.attempts.push(ReviewAttempt {
                        message: input.to_string(),
                        accepted: judgment.accepted,
                        feedback: judgment.feedback.clone(),
                    });

                    if judgment.accepted {
                        *session = LearnerSession::Replying(ReplyState::new(
                            review.message.clone(),
                            UnderstandingSummary::TranslatedCorrectly {
                                translation_attempts: review.attempts.len(),
                            },
                        ));
                        turn.learner_messages.push("Correct!".into());
                        turn.learner_messages.push(format!(
                            "Write your reply in {}:",
                            llm.target_language()
                        ));
                    } else {
                        let teaching = llm
                            .teach_message_with_tips(&review.message, input, history)
                            .await?;
                        *session = LearnerSession::Replying(ReplyState::new(
                            review.message.clone(),
                            UnderstandingSummary::TranslatedIncorrectly {
                                translation_attempts: review.attempts.len(),
                            },
                        ));
                        turn.learner_messages.push(teaching);
                        turn.learner_messages.push(format!(
                            "Write your reply in {}:",
                            llm.target_language()
                        ));
                    }
                    break;
                }
            },
            LearnerSession::Replying(reply) => {
                let judgment = llm.validate_reply(input, history).await?;
                reply.attempts.push(ReplyAttempt {
                    message: input.to_string(),
                    accepted: judgment.accepted,
                    feedback: judgment.feedback.clone(),
                });

                if judgment.accepted {
                    let summary = build_teacher_summary(reply, input);
                    turn.completed_reply = Some(input.to_string());
                    turn.teacher_message = Some(summary);
                    *session = LearnerSession::Idle;
                } else {
                    turn.learner_messages.push(judgment.feedback);
                }
                break;
            }
        }
    }

    Ok(turn)
}

pub fn build_teacher_summary(reply: &ReplyState, learner_message: &str) -> String {
    let understanding = match reply.understanding {
        UnderstandingSummary::UnderstoodCompletely => {
            "The learner understood your message completely.".to_string()
        }
        UnderstandingSummary::DidNotUnderstand => {
            "The learner didn't understand your message and was given a full explanation."
                .to_string()
        }
        UnderstandingSummary::TranslatedCorrectly {
            translation_attempts,
        } => {
            if translation_attempts == 1 {
                "The learner guessed the meaning of your message and got it right on first try."
                    .to_string()
            } else {
                format!(
                    "The learner guessed the meaning of your message correctly after {translation_attempts} attempts."
                )
            }
        }
        UnderstandingSummary::TranslatedIncorrectly {
            translation_attempts,
        } => format!(
            "The learner tried to translate your message {translation_attempts} time(s) before being taught the meaning."
        ),
    };

    let reply_attempts = reply.attempts.len();

    format!(
        "Reply from learner: {learner_message}\n\n{understanding}\nThe learner needed {reply_attempts} iteration(s) to craft this message."
    )
}
