mod llm;

use std::io::{self, Write};

use anyhow::Context;

use llm::Llm;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("molvakt — language learning bot (CLI prototype)\n");

    let llm = Llm::from_env().context("failed to initialize OpenAI client")?;
    let mut app = App::new();

    loop {
        match &mut app.state {
            State::Waiting => {
                let message = read_line("Alice sends: ");
                if message.is_empty() {
                    continue;
                }
                let judgment = llm.validate_alice_message(&message).await?;
                if !judgment.accepted {
                    println!("\nRejected: {}\n", judgment.feedback);
                    continue;
                }
                app.history.push(HistoryEntry::Alice(message.clone()));
                app.state = State::Reviewing(ReviewState::new(message));
            }
            State::Reviewing(review) => {
                if let Some(reply_state) = handle_reviewing(review, &app.history, &llm).await? {
                    app.state = State::Replying(reply_state);
                }
            }
            State::Replying(reply) => {
                if let Some(bob_message) = handle_replying(reply, &app.history, &llm).await? {
                    let summary = build_alice_summary(reply, &bob_message);
                    app.history.push(HistoryEntry::Bob(bob_message.clone()));
                    println!("\n--- Message sent to Alice ---\n{summary}\n");
                    app.state = State::Waiting;
                }
            }
        }
    }
}

struct App {
    history: Vec<HistoryEntry>,
    state: State,
}

enum HistoryEntry {
    Alice(String),
    Bob(String),
}

enum State {
    Waiting,
    Reviewing(ReviewState),
    Replying(ReplyState),
}

struct ReviewState {
    message: String,
    phase: ReviewPhase,
    attempts: Vec<ReviewAttempt>,
}

enum ReviewPhase {
    Choosing,
    Teaching,
    Quizzing,
}

struct ReviewAttempt {
    message: String,
    feedback: Result<(), String>,
}

struct ReplyState {
    alice_message: String,
    understanding: UnderstandingSummary,
    attempts: Vec<ReplyAttempt>,
}

enum UnderstandingSummary {
    UnderstoodCompletely,
    DidNotUnderstand,
    TranslatedCorrectly { translation_attempts: usize },
    TranslatedIncorrectly { translation_attempts: usize },
}

struct ReplyAttempt {
    message: String,
    feedback: Result<(), String>,
}

impl ReviewState {
    fn new(message: String) -> Self {
        Self {
            message,
            phase: ReviewPhase::Choosing,
            attempts: Vec::new(),
        }
    }
}

impl ReplyState {
    fn new(alice_message: String, understanding: UnderstandingSummary) -> Self {
        Self {
            alice_message,
            understanding,
            attempts: Vec::new(),
        }
    }
}

impl App {
    fn new() -> Self {
        Self {
            history: Vec::new(),
            state: State::Waiting,
        }
    }
}

async fn handle_reviewing(
    review: &mut ReviewState,
    history: &[HistoryEntry],
    llm: &Llm,
) -> anyhow::Result<Option<ReplyState>> {
    println!("\nMessage from Alice: \"{}\"", review.message);

    match review.phase {
        ReviewPhase::Choosing => {
            println!("\nHow well did you understand this message?");
            println!("  1. I understand completely");
            println!("  2. I don't understand");
            println!("  3. I might understand");

            let choice = read_line("> ");
            match choice.as_str() {
                "1" => {
                    return Ok(Some(ReplyState::new(
                        review.message.clone(),
                        UnderstandingSummary::UnderstoodCompletely,
                    )));
                }
                "2" => {
                    review.phase = ReviewPhase::Teaching;
                }
                "3" => {
                    review.phase = ReviewPhase::Quizzing;
                    println!("\nTry to translate the message into English.");
                }
                _ => println!("Invalid choice, pick 1, 2, or 3."),
            }
        }
        ReviewPhase::Teaching => {
            let teaching = llm.teach_message(&review.message, history).await?;
            println!("\n{teaching}");
            return Ok(Some(ReplyState::new(
                review.message.clone(),
                UnderstandingSummary::DidNotUnderstand,
            )));
        }
        ReviewPhase::Quizzing => {
            let attempt = read_line("Your translation: ");
            let judgment = llm
                .rate_review_attempt(&attempt, &review.message, history)
                .await?;
            let result = ReviewAttempt {
                message: attempt,
                feedback: judgment_to_result(judgment.accepted, judgment.feedback),
            };
            let success = result.feedback.is_ok();
            review.attempts.push(result);

            if success {
                println!("\nCorrect!");
                return Ok(Some(ReplyState::new(
                    review.message.clone(),
                    UnderstandingSummary::TranslatedCorrectly {
                        translation_attempts: review.attempts.len(),
                    },
                )));
            }

            let wrong = review.attempts.last().unwrap().message.clone();
            let teaching = llm
                .teach_message_with_tips(&review.message, &wrong, history)
                .await?;
            println!("\n{teaching}");
            return Ok(Some(ReplyState::new(
                review.message.clone(),
                UnderstandingSummary::TranslatedIncorrectly {
                    translation_attempts: review.attempts.len(),
                },
            )));
        }
    }

    Ok(None)
}

async fn handle_replying(
    reply: &mut ReplyState,
    history: &[HistoryEntry],
    llm: &Llm,
) -> anyhow::Result<Option<String>> {
    if reply.attempts.is_empty() {
        println!("\nWrite your reply in the target language:");
    }

    let attempt = read_line("> ");
    let judgment = llm.validate_reply(&attempt, history).await?;
    let result = ReplyAttempt {
        message: attempt.clone(),
        feedback: judgment_to_result(judgment.accepted, judgment.feedback),
    };
    reply.attempts.push(result);

    match reply.attempts.last().unwrap().feedback {
        Ok(()) => Ok(Some(attempt)),
        Err(ref feedback) => {
            println!("\n{feedback}");
            Ok(None)
        }
    }
}

fn judgment_to_result(accepted: bool, feedback: String) -> Result<(), String> {
    if accepted {
        Ok(())
    } else {
        Err(feedback)
    }
}

fn build_alice_summary(reply: &ReplyState, bob_message: &str) -> String {
    let understanding = match reply.understanding {
        UnderstandingSummary::UnderstoodCompletely => {
            "Bob understood your message completely.".to_string()
        }
        UnderstandingSummary::DidNotUnderstand => {
            "Bob didn't understand your message and was given a full explanation.".to_string()
        }
        UnderstandingSummary::TranslatedCorrectly {
            translation_attempts,
        } => {
            if translation_attempts == 1 {
                "Bob guessed the meaning of your message and got it right on first try."
                    .to_string()
            } else {
                format!(
                    "Bob guessed the meaning of your message correctly after {translation_attempts} attempts."
                )
            }
        }
        UnderstandingSummary::TranslatedIncorrectly {
            translation_attempts,
        } => format!(
            "Bob tried to translate your message {translation_attempts} time(s) before being taught the meaning."
        ),
    };

    let reply_attempts = reply.attempts.len();

    format!(
        "Reply from Bob: {bob_message}\n\n{understanding}\nBob needed {reply_attempts} iteration(s) to craft this message."
    )
}

fn read_line(prompt: &str) -> String {
    print!("{prompt}");
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    input.trim().to_string()
}
