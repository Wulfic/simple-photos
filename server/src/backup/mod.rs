//! Backup system: server-to-server replication, recovery, auto-discovery,
//! and periodic storage directory scanning.
//!
//! Sub-modules:
//! - [`handlers`]  — Admin CRUD for backup servers plus mode/audio settings.
//! - [`serve`]     — API-key-authenticated endpoints that expose this server's
//!   photos to other servers (list, list-trash, download, receive with checksum
//!   verification).
//! - [`sync`]      — Push-based sync engine with delta transfer, per-server
//!   concurrency lock, SHA-256 checksums, and per-file error tracking.
//! - [`recovery`]  — Pull-based recovery that downloads missing photos from
//!   a backup server (deduplicates by photo ID).
//! - [`broadcast`] — UDP beacon for LAN auto-discovery of backup-mode servers.
//! - [`autoscan`]  — Background task that registers new files found on disk.

pub mod autoscan;
pub mod broadcast;
pub mod handlers;
pub mod models;
pub mod recovery;
pub mod serve;
pub mod sync;
