//! Backup system: server-to-server replication, recovery, auto-discovery,
//! and periodic storage directory scanning.
//!
//! Sub-modules:
//! - [`handlers`]    — Admin CRUD for backup servers plus mode/audio settings.
//! - [`serve`]       — API-key-authenticated endpoints that expose this server's
//!   photos to other servers (list, list-trash, download, receive with checksum
//!   verification).
//! - [`sync`]        — Push-based sync engine with delta transfer, per-server
//!   concurrency lock, SHA-256 checksums, and per-file error tracking.
//! - [`recovery`]    — Pull-based recovery that downloads missing photos from
//!   a backup server (deduplicates by photo ID).
//! - [`broadcast`]   — UDP beacon for LAN auto-discovery of backup-mode servers.
//! - [`autoscan`]    — Background task that registers new files found on disk.
//! - [`diagnostics`] — One-way health reporting: backup pushes snapshots to the
//!   primary every 15 min; primary stores and exposes them via the admin API.
//!   Android clients are blocked from submitting diagnostics to backup servers.

pub mod autoscan;
pub mod broadcast;
pub mod diagnostics;
pub mod discovery;
pub mod handlers;
pub mod models;
pub mod recovery;
pub mod serve;
pub mod sync;
