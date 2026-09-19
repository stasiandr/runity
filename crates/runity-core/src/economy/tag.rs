//! Material properties, per `05-economy.md` §6.1: recipes and stakes name a
//! property and an amount, never a specific material.

/// A material property from `05-economy.md` §6.2. Nine does not get added
/// casually: each tag multiplies the substitutions a recipe writer has to
/// think through, not just lengthen a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tag {
    /// Winds, ties, holds a knot — nettle fibre, bast, sinew.
    Fibrous,
    /// Burns on its own — a branch, a plank, tallow.
    Flammable,
    /// Holds shape under load — a stick, a log, bone.
    Hard,
    /// Cuts and chops — knapped flint, a bone splinter, an iron blade.
    Sharp,
    /// Holds heat — moss, hay, fur.
    Warm,
    /// Soaks in, clings, burns long — resin, rendered fat, wax.
    Oily,
    /// Takes shape from a blow while hot — bloom iron, iron, lead.
    Malleable,
    /// Food.
    Edible,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_compare_and_hash_by_value() {
        assert_eq!(Tag::Hard, Tag::Hard);
        assert_ne!(Tag::Hard, Tag::Sharp);
    }
}
