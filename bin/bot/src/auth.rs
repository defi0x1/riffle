// The single choke point every incoming chat has to pass before a command runs. A leaked bot
// token still cannot pull anything out of a chat that is not on this list.
pub fn is_authorized(chat_id: i64, allowed: &[i64]) -> bool {
    allowed.contains(&chat_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_listed_chat_is_authorized() {
        assert!(is_authorized(42, &[1, 42, 99]));
    }

    #[test]
    fn test_unlisted_chat_is_refused() {
        assert!(!is_authorized(7, &[1, 42, 99]));
    }

    #[test]
    fn test_empty_allow_list_refuses_everyone() {
        assert!(!is_authorized(1, &[]));
    }
}
