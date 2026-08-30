mod bot;
mod cli;
mod conversations;
mod db;
mod flow;
mod history;
mod llm;
mod onboarding;
mod phone;
mod server;
mod whatsapp;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::args().any(|arg| arg == "--cli") {
        cli::run().await
    } else {
        server::run().await
    }
}
