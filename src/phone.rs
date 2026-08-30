pub fn normalize_phone(phone: &str) -> String {
    phone.chars().filter(|c| c.is_ascii_digit()).collect()
}

pub fn phones_match(left: &str, right: &str) -> bool {
    normalize_phone(left) == normalize_phone(right)
}

pub fn looks_like_phone(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let digits = normalize_phone(trimmed);
    digits.len() >= 8 && digits.len() * 2 >= trimmed.len()
}

pub fn partner_label(phone: &str, language: &str) -> String {
    format!("{} ({language})", display_phone(phone))
}

pub fn display_phone(phone: &str) -> String {
    let digits = normalize_phone(phone);
    if digits.is_empty() {
        return phone.to_string();
    }
    format!("+{digits}")
}
