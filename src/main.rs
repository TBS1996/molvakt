use std::io::{self, Write};

fn main() {
    println!("molvakt — language learning bot (CLI prototype)\n");

    let mut app = App::new();

    loop {
        match &mut app.state {
            State::Waiting => {
                let message = read_line("Alice sends: ");
                if message.is_empty() {
                    continue;
                }
                app.history.push(HistoryEntry::Alice(message.clone()));
                app.state = State::Reviewing(ReviewState::new(message));
            }
            State::Reviewing(review) => {
                if let Some(reply_state) = handle_reviewing(review, &app.history) {
                    app.state = State::Replying(reply_state);
                }
            }
            State::Replying(reply) => {
                if let Some(bob_message) = handle_replying(reply, &app.history) {
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

fn handle_reviewing(
    review: &mut ReviewState,
    history: &[HistoryEntry],
) -> Option<ReplyState> {
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
                    return Some(ReplyState::new(
                        review.message.clone(),
                        UnderstandingSummary::UnderstoodCompletely,
                    ));
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
            let teaching = teach_message(&review.message, history);
            println!("\n{teaching}");
            return Some(ReplyState::new(
                review.message.clone(),
                UnderstandingSummary::DidNotUnderstand,
            ));
        }
        ReviewPhase::Quizzing => {
            let attempt = read_line("Your translation: ");
            let result = rate_review_attempt(&attempt, &review.message, history);
            let success = result.feedback.is_ok();
            review.attempts.push(result);

            if success {
                println!("\nCorrect!");
                return Some(ReplyState::new(
                    review.message.clone(),
                    UnderstandingSummary::TranslatedCorrectly {
                        translation_attempts: review.attempts.len(),
                    },
                ));
            }

            let wrong = review.attempts.last().unwrap().message.clone();
            let teaching = teach_message_with_tips(&review.message, &wrong, history);
            println!("\n{teaching}");
            return Some(ReplyState::new(
                review.message.clone(),
                UnderstandingSummary::TranslatedIncorrectly {
                    translation_attempts: review.attempts.len(),
                },
            ));
        }
    }

    None
}

fn handle_replying(reply: &mut ReplyState, history: &[HistoryEntry]) -> Option<String> {
    if reply.attempts.is_empty() {
        println!("\nWrite your reply in the target language:");
    }

    let attempt = read_line("> ");
    let result = validate_reply(&attempt, history);
    reply.attempts.push(result);

    match reply.attempts.last().unwrap().feedback {
        Ok(()) => Some(attempt),
        Err(ref feedback) => {
            println!("\n{feedback}");
            None
        }
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

// --- Stub LLM calls (replace with real API later) ---

fn rate_review_attempt(
    attempt: &str,
    original: &str,
    _history: &[HistoryEntry],
) -> ReviewAttempt {
    let attempt_lower = attempt.to_lowercase();

    // Stub: accept translations that mention "how are you" for the demo message
    let accepted = attempt_lower.contains("how are you")
        || attempt_lower.contains("how're you")
        || (original.contains("hvordan") && attempt_lower.contains("how"));

    let feedback = if accepted {
        Ok(())
    } else {
        Err("Not quite right. Think about what greeting or question is being asked.".to_string())
    };

    ReviewAttempt {
        message: attempt.to_string(),
        feedback,
    }
}

fn teach_message(original: &str, _history: &[HistoryEntry]) -> String {
    format!(
        "Translation: {}\n\n\
         Grammar: This is a common Norwegian greeting/question.\n\
         Words:\n\
           - \"Hei\" = Hi\n\
           - \"hvordan\" = how\n\
           - \"har du det\" = are you (doing)\n\
           - \"?\" = question",
        stub_translation(original)
    )
}

fn teach_message_with_tips(
    original: &str,
    wrong_attempt: &str,
    history: &[HistoryEntry],
) -> String {
    let mut teaching = teach_message(original, history);
    teaching.push_str(&format!(
        "\n\nYour attempt (\"{wrong_attempt}\") was close but not quite right. \
         Pay attention to the question word and how Norwegian orders the phrase."
    ));
    teaching
}

fn validate_reply(attempt: &str, _history: &[HistoryEntry]) -> ReplyAttempt {
    let attempt_lower = attempt.to_lowercase();

    let has_error = attempt_lower.contains("i am")
        || attempt_lower.contains("thank you for asking")
        || !attempt.chars().any(|c| c.is_ascii_alphabetic());

    let feedback = if has_error {
        Err(
            "Try writing in Norwegian. Hint: use \"jeg\" instead of \"I am\", \
             and \"takk som spør\" instead of \"thank you for asking\"."
                .to_string(),
        )
    } else if !attempt.ends_with('?') && attempt_lower.contains("hvordan") {
        Err("Remember the question mark at the end.".to_string())
    } else {
        Ok(())
    };

    ReplyAttempt {
        message: attempt.to_string(),
        feedback,
    }
}

fn stub_translation(original: &str) -> &'static str {
    if original.to_lowercase().contains("hvordan") {
        "Hi, how are you?"
    } else {
        "(translation unavailable in stub)"
    }
}
