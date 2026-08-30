use std::io::{self, Write};

use anyhow::Context;

use crate::db::{Db, MessageRole};
use crate::flow::{self, LearnerSession};
use crate::llm::Llm;

pub async fn run() -> anyhow::Result<()> {
    println!("molvakt — language learning bot (CLI prototype)\n");

    let db = Db::connect().await?;
    let target_language = std::env::var("MOLVAKT_TARGET_LANGUAGE")
        .unwrap_or_else(|_| "Norwegian".into());
    let source_language = std::env::var("MOLVAKT_SOURCE_LANGUAGE")
        .unwrap_or_else(|_| "English".into());
    let conversation = db
        .create_conversation(&target_language, &source_language)
        .await?;
    let llm = Llm::from_env(&conversation).context("failed to initialize OpenAI client")?;
    let history = db.load_history(conversation.id).await?;

    println!(
        "Learning {} (translations in {})\n",
        conversation.target_language, conversation.source_language
    );
    if !history.is_empty() {
        println!("Loaded {} message(s) from previous session.\n", history.len());
    }

    let mut session = LearnerSession::Idle;

    loop {
        match &session {
            LearnerSession::Idle => {
                let message = read_line("Teacher sends: ");
                if message.is_empty() {
                    continue;
                }
                let judgment = llm.validate_teacher_message(&message).await?;
                if !judgment.accepted {
                    println!("\nRejected: {}\n", judgment.feedback);
                    continue;
                }
                db.insert_message(conversation.id, MessageRole::Teacher, &message)
                    .await?;
                let (new_session, teacher_message) =
                    flow::begin_review(message, "teacher (CLI)");
                session = new_session;
                println!("\n{teacher_message}");
                println!("\n{}", flow::review_choice_prompt());
            }
            LearnerSession::Reviewing(_) | LearnerSession::Replying(_) => {
                let input = read_line("> ");
                let current_history = db.load_history(conversation.id).await?;
                let turn = flow::handle_learner_message(
                    &mut session,
                    &input,
                    &current_history,
                    &llm,
                    "learner (CLI)",
                )
                .await?;

                for message in turn.learner_messages {
                    println!("\n{message}");
                }

                if let Some(learner_reply) = turn.completed_reply {
                    db.insert_message(conversation.id, MessageRole::Learner, &learner_reply)
                        .await?;
                    if let Some(summary) = turn.teacher_message {
                        println!("\n--- Message sent to teacher ---\n{summary}\n");
                    }
                    session = LearnerSession::Idle;
                }
            }
        }
    }
}

fn read_line(prompt: &str) -> String {
    print!("{prompt}");
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    input.trim().to_string()
}
