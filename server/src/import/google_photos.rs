//! Google Photos metadata parsing and normalisation.
//!
//! Parses the JSON sidecar files that Google Photos Takeout exports alongside
//! each media file. These contain timestamps, GPS coordinates, view counts, and
//! the original Google Photos URL.

use chrono::{DateTime, TimeZone, Utc};

use super::models::{GooglePhotosMetadata, PhotoMetadataRecord};

/// Parse a Google Photos JSON sidecar file from raw bytes.
pub fn parse_sidecar(data: &[u8]) -> Result<GooglePhotosMetadata, String> {
    serde_json::from_slice(data).map_err(|e| format!("Invalid Google Photos JSON: {}", e))
}

/// Convert a Google Photos metadata struct into a normalised `PhotoMetadataRecord`.
/// The caller must provide `id`, `user_id`, and optionally `photo_id`/`blob_id`.
pub fn normalise(
    meta: &GooglePhotosMetadata,
    id: String,
    user_id: String,
    photo_id: Option<String>,
    blob_id: Option<String>,
) -> PhotoMetadataRecord {
    // Parse photoTakenTime → ISO 8601
    let taken_at = meta
        .photo_taken_time
        .as_ref()
        .and_then(|t| timestamp_to_rfc3339(t.timestamp.as_deref()));

    // Parse creationTime → ISO 8601
    let created_at_src = meta
        .creation_time
        .as_ref()
        .and_then(|t| timestamp_to_rfc3339(t.timestamp.as_deref()));

    // Parse geo data — Google Photos exports (0.0, 0.0) when no location is set;
    // treat that as absent.
    let (latitude, longitude, altitude) = meta
        .geo_data
        .as_ref()
        .map(|g| {
            let lat = g.latitude.filter(|&v| v.abs() > f64::EPSILON);
            let lng = g.longitude.filter(|&v| v.abs() > f64::EPSILON);
            let alt = g.altitude.filter(|&v| v.abs() > f64::EPSILON);
            (lat, lng, alt)
        })
        .unwrap_or((None, None, None));

    let image_views = meta
        .image_views
        .as_ref()
        .and_then(|v| v.parse::<i64>().ok());

    let now = Utc::now().to_rfc3339();

    PhotoMetadataRecord {
        id,
        user_id,
        photo_id,
        blob_id,
        source: "google_photos".to_string(),
        title: meta.title.clone(),
        description: meta.description.clone().filter(|d| !d.is_empty()),
        taken_at,
        created_at_src,
        latitude,
        longitude,
        altitude,
        image_views,
        original_url: meta.url.clone(),
        storage_path: None,
        is_encrypted: false,
        imported_at: now,
    }
}

/// Convert a Unix timestamp string (seconds since epoch) to RFC 3339.
fn timestamp_to_rfc3339(ts: Option<&str>) -> Option<String> {
    ts.and_then(|s| s.parse::<i64>().ok())
        .and_then(|secs| Utc.timestamp_opt(secs, 0).single())
        .map(|dt: DateTime<Utc>| dt.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_google_photos_sidecar() {
        let json = r#"{
            "title": "1.jpg",
            "description": "",
            "imageViews": "383",
            "creationTime": {
                "timestamp": "1495581900",
                "formatted": "May 23, 2017, 11:25:00 PM UTC"
            },
            "photoTakenTime": {
                "timestamp": "1494963474",
                "formatted": "May 16, 2017, 7:37:54 PM UTC"
            },
            "geoData": {
                "latitude": 0.0,
                "longitude": 0.0,
                "altitude": 0.0,
                "latitudeSpan": 0.0,
                "longitudeSpan": 0.0
            },
            "url": "https://photos.google.com/photo/AF1QipOgKycyqC-KDoZ1U8XHFqqSWrhBm7VGSu1T3odO",
            "googlePhotosOrigin": { "driveSync": {} }
        }"#;

        let meta = parse_sidecar(json.as_bytes()).unwrap();
        assert_eq!(meta.title.as_deref(), Some("1.jpg"));
        assert_eq!(meta.image_views.as_deref(), Some("383"));

        let record = normalise(
            &meta,
            "test-id".to_string(),
            "user-1".to_string(),
            Some("photo-1".to_string()),
            None,
        );

        assert_eq!(record.source, "google_photos");
        assert_eq!(record.image_views, Some(383));
        assert!(record.taken_at.is_some());
        // (0,0) coordinates should be treated as absent
        assert!(record.latitude.is_none());
        assert!(record.longitude.is_none());
    }
}
