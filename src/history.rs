#[derive(Clone)]
pub enum HistoryEntry {
    Teacher(String),
    Learner(String),
    Exchange { sender: String, content: String },
}
