use std::collections::HashSet;

// Muting is now a row in `muted_pools` (storage::write::mute_pool /
// storage::queries::muted_pool_addresses), keyed by (pool_address, chat_id), with the SQL
// predicate `until > now()` doing expiry -- nothing here needs a clock, a sweeper, or process
// memory anymore. The one piece of logic still local to this binary is turning a fetched set
// of muted addresses into a per-row tag for rendering, which stays pure and database-free.
pub fn tag_muted<T>(
    rows: Vec<T>,
    muted: &HashSet<String>,
    address_of: impl Fn(&T) -> &str,
) -> Vec<(T, bool)> {
    rows.into_iter()
        .map(|row| {
            let is_muted = muted.contains(address_of(&row));
            (row, is_muted)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct Row(String);

    #[test]
    fn test_tags_a_muted_row() {
        let muted: HashSet<String> = ["pool1".to_string()].into_iter().collect();
        let rows = vec![Row("pool1".to_string())];

        let tagged = tag_muted(rows, &muted, |r| &r.0);

        assert_eq!(tagged, vec![(Row("pool1".to_string()), true)]);
    }

    #[test]
    fn test_does_not_tag_an_unmuted_row() {
        let muted: HashSet<String> = HashSet::new();
        let rows = vec![Row("pool1".to_string())];

        let tagged = tag_muted(rows, &muted, |r| &r.0);

        assert_eq!(tagged, vec![(Row("pool1".to_string()), false)]);
    }

    #[test]
    fn test_tags_rows_independently_within_the_same_batch() {
        let muted: HashSet<String> = ["pool2".to_string()].into_iter().collect();
        let rows = vec![Row("pool1".to_string()), Row("pool2".to_string())];

        let tagged = tag_muted(rows, &muted, |r| &r.0);

        assert_eq!(
            tagged,
            vec![
                (Row("pool1".to_string()), false),
                (Row("pool2".to_string()), true),
            ]
        );
    }
}
