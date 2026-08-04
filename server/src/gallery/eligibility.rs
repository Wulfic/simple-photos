//! The single definition of "is this photo in the gallery feed?".
//!
//! Eligibility is not a column — it is a set of subqueries against
//! `encrypted_gallery_items`. That has two consequences worth stating plainly,
//! because both have produced bugs in this repo:
//!
//! 1. **A photo can leave the feed without its row being touched.** Adding it
//!    to a secure album inserts an `encrypted_gallery_items` row and nothing
//!    else. Anything that watches only `photos` will miss it — see the EGI
//!    triggers in `migrations/033_photo_change_log.sql`.
//! 2. **The predicate was copy-pasted.** It appeared verbatim in the two
//!    `gallery::sync` queries and again in `gallery::summary`. Delta sync adds
//!    two more uses, and a delta feed whose notion of eligibility differs from
//!    the full walk's — by even one arm — hands clients rows the grid will
//!    never show, or hides rows it will. Hence one const, interpolated.
//!
//! All callers must alias the table as `p` (`FROM photos p`). The qualifier is
//! not decoration: the delta query joins `photo_change_log`, which also has a
//! `user_id`, so unqualified column names would be ambiguous.

/// Rows the gallery feed returns: everything not claimed by a secure gallery.
///
/// The three arms cover the three ways a secure-gallery item can name a photo:
/// by the photo's own id, by the id it was cloned from, or indirectly through
/// the photo's encrypted blob. `migrations/033_photo_change_log.sql` mirrors
/// this three-way match in its EGI triggers — grow one, grow the other.
///
/// Deliberately does NOT constrain `encrypted_blob_id`. Rows still awaiting
/// encryption are eligible and are counted (#42: the count is
/// server-authoritative and includes the pending-encryption backlog).
pub const ELIGIBLE_PREDICATE: &str = "p.id NOT IN (SELECT blob_id FROM encrypted_gallery_items) \
     AND p.id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL) \
     AND (p.encrypted_blob_id IS NULL OR p.encrypted_blob_id NOT IN (SELECT original_blob_id FROM encrypted_gallery_items WHERE original_blob_id IS NOT NULL))";

/// `WHERE` body selecting one user's eligible rows. `?1` is the user id.
pub fn eligible_for_user() -> String {
    format!("p.user_id = ?1 AND {ELIGIBLE_PREDICATE}")
}

// NOTE: there is deliberately no SQL helper for the *negation*. The delta feed
// derives tombstones by set-difference in Rust — it pages the change log, asks
// which of that page's ids are still eligible, and treats the remainder as
// removed. That yields "deleted" and "secure-hidden" from one query instead of
// two mirrored predicates that could disagree, and it keeps the page boundary
// and the eligibility split governed by the same row set.

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards the aliasing contract. If someone drops the `p.` qualifiers the
    /// delta query stops compiling at runtime (ambiguous `user_id`) — in a
    /// query built by `format!`, so the compiler will not catch it.
    #[test]
    fn predicate_qualifies_every_photos_column() {
        for frag in ["p.id NOT IN", "p.encrypted_blob_id"] {
            assert!(
                ELIGIBLE_PREDICATE.contains(frag),
                "predicate must qualify photos columns with the `p` alias; missing {frag}"
            );
        }
        assert!(eligible_for_user().starts_with("p.user_id = ?1"));
    }

    /// The three arms are load-bearing and mirrored by the EGI triggers in
    /// migration 033. Dropping one here silently widens the feed to include
    /// secure-gallery photos — a confidentiality bug, not a counting one.
    #[test]
    fn predicate_covers_all_three_secure_gallery_references() {
        assert_eq!(
            ELIGIBLE_PREDICATE
                .matches("encrypted_gallery_items")
                .count(),
            3,
            "expected exactly three secure-gallery arms; if this changed on \
             purpose, update the EGI triggers in migration 033 to match"
        );
    }
}
