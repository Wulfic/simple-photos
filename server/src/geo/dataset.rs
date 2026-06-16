//! On-demand download of the offline GeoNames reverse-geocoding dataset.
//!
//! The installers fetch `cities500.txt` post-install, but that step can fail
//! (a network blip, DNS, or a service account that has no internet at install
//! time) and there was previously no recovery short of re-running the whole
//! installer — reverse geocoding simply stayed dead, with the client banner
//! reporting "location data unavailable" forever.
//!
//! This module lets the running server fetch the dataset itself the first time
//! a user actually enables geo and a photo needs resolving. It self-heals a
//! failed install on every platform with no reinstall, and the geo processor
//! picks the freshly-downloaded dataset up on its next poll cycle.
//!
//! Network egress here only happens when geolocation is *enabled* (a deliberate
//! opt-in) AND the dataset is missing AND there is at least one photo pending
//! resolution — the same gate the processor already uses to decide it needs the
//! geocoder at all.

use std::path::Path;
use std::time::Duration;

/// GeoNames cities500 archive (≈25 MB zip, ≈70 MB extracted). Same URL the
/// installers use, so behaviour is identical whether the dataset arrives via
/// the installer or this runtime fallback.
const CITIES_ZIP_URL: &str = "https://download.geonames.org/export/dump/cities500.zip";
/// Companion file mapping `<cc>.<adm1>` → full region name ("US.CA" →
/// "California"). Best-effort: without it state names fall back to raw codes.
const ADMIN1_URL: &str = "https://download.geonames.org/export/dump/admin1CodesASCII.txt";

/// Outcome of an [`ensure_dataset`] attempt.
pub enum FetchOutcome {
    /// The dataset file is present (already was, or was just downloaded).
    Present,
    /// A download was attempted and failed — retry on a later cycle.
    Failed(String),
}

/// Ensure `dataset_path` (cities500.txt) exists, downloading and extracting it
/// from GeoNames when missing. Also best-effort fetches the admin1 companion
/// file next to it. Safe to call repeatedly — returns [`FetchOutcome::Present`]
/// immediately once the file is on disk.
pub async fn ensure_dataset(dataset_path: &str, user_agent: &str) -> FetchOutcome {
    if Path::new(dataset_path).exists() {
        return FetchOutcome::Present;
    }
    match download_and_extract(dataset_path, user_agent).await {
        Ok(()) => FetchOutcome::Present,
        Err(e) => FetchOutcome::Failed(e),
    }
}

async fn download_and_extract(dataset_path: &str, user_agent: &str) -> Result<(), String> {
    let dest = Path::new(dataset_path).to_path_buf();
    // `Path::parent()` is `Some("")` for a bare filename (cwd-relative
    // dataset_path) — creating "" errors, so only mkdir a non-empty parent.
    let parent = dest
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    if !parent.as_os_str().is_empty() {
        tokio::fs::create_dir_all(&parent)
            .await
            .map_err(|e| format!("create data dir '{}': {e}", parent.display()))?;
    }

    let client = reqwest::Client::builder()
        .user_agent(user_agent.to_string())
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| format!("build geo dataset http client: {e}"))?;

    // ── 1. cities500.zip → extract cities500.txt ────────────────────────────
    tracing::info!(url = CITIES_ZIP_URL, "Downloading GeoNames cities500 dataset (runtime fallback)");
    let resp = client
        .get(CITIES_ZIP_URL)
        .send()
        .await
        .map_err(|e| format!("download cities500.zip: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("download cities500.zip: HTTP {}", resp.status()));
    }
    let zip_bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("read cities500.zip body: {e}"))?
        .to_vec();

    // Zip inflation is synchronous + CPU-bound — do it off the async runtime.
    let dest_for_blocking = dest.clone();
    tokio::task::spawn_blocking(move || extract_cities_txt(zip_bytes, &dest_for_blocking))
        .await
        .map_err(|e| format!("dataset extract task panicked: {e}"))??;

    tracing::info!(path = %dest.display(), "GeoNames cities500 dataset installed");

    // ── 2. admin1CodesASCII.txt (best-effort — only improves state names) ────
    let admin1 = parent.join("admin1CodesASCII.txt");
    if !admin1.exists() {
        match client.get(ADMIN1_URL).send().await {
            Ok(r) if r.status().is_success() => match r.bytes().await {
                Ok(b) => {
                    if let Err(e) = tokio::fs::write(&admin1, &b).await {
                        tracing::warn!(error = %e, "admin1 write failed — state names fall back to codes");
                    }
                }
                Err(e) => tracing::warn!(error = %e, "admin1 read failed"),
            },
            Ok(r) => tracing::warn!(status = %r.status(), "admin1 download returned non-success"),
            Err(e) => tracing::warn!(error = %e, "admin1 download failed"),
        }
    }

    Ok(())
}

/// Extract `cities500.txt` from the in-memory zip to `dest`, writing through a
/// `.part` temp file + rename so a partial extraction never looks complete.
fn extract_cities_txt(zip_bytes: Vec<u8>, dest: &Path) -> Result<(), String> {
    let reader = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(reader).map_err(|e| format!("open cities500.zip: {e}"))?;
    let mut entry = archive
        .by_name("cities500.txt")
        .map_err(|e| format!("cities500.txt not found in archive: {e}"))?;

    let tmp = dest.with_extension("txt.part");
    {
        let mut out =
            std::fs::File::create(&tmp).map_err(|e| format!("create temp dataset file: {e}"))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| format!("extract cities500.txt: {e}"))?;
    }
    std::fs::rename(&tmp, dest).map_err(|e| format!("finalize dataset file: {e}"))?;
    Ok(())
}
