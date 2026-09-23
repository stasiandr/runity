//! The closest known name to a mistyped one.
//!
//! Every place a name can dangle — a model, a material, a prefab, a
//! component — answers "did you mean …?" the same way, so the answer is
//! here once.

/// The known name nearest to `wanted`, if one is close enough to be a typo:
/// within a third of its length, and never more than that or less than two
/// edits.
pub fn closest<'a>(wanted: &str, known: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let limit = (wanted.chars().count() / 3).max(2);
    known
        .into_iter()
        .map(|name| (distance(wanted, name), name))
        .filter(|(d, _)| *d <= limit)
        .min()
        .map(|(_, name)| name)
}

/// Levenshtein distance, by characters.
pub fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitute = previous + usize::from(ca != *cb);
            previous = row[j + 1];
            row[j + 1] = substitute.min(row[j] + 1).min(previous + 1);
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typo_finds_its_word_and_a_stranger_finds_nothing() {
        let known = ["rock", "pine_large", "campfire"];
        assert_eq!(closest("rok", known), Some("rock"));
        assert_eq!(closest("pine_lrage", known), Some("pine_large"));
        assert_eq!(closest("helicopter", known), None);
        assert_eq!(distance("kitten", "sitting"), 3);
    }
}
