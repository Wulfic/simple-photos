//! Trash (soft-delete) system with 30-day retention.
//!
//! Deleted photos are moved to the `trash_items` table instead of being
//! permanently removed. A background task purges items older than 30 days.
//! Users can restore or permanently delete individual items.

pub mod handlers;
pub mod models;
