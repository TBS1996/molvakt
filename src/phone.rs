pub fn normalize_phone(phone: &str) -> String {
    phone.chars().filter(|c| c.is_ascii_digit()).collect()
}

pub fn display_phone(phone: &str) -> String {
    let digits = normalize_phone(phone);
    if digits.is_empty() {
        return phone.to_string();
    }
    format!("+{digits}")
}
