package com.simplephotos.data

import android.util.Base64
import org.json.JSONObject

/**
 * Decode the `{ "data": "<base64>" }` envelope that wraps a decrypted thumbnail
 * blob into raw image bytes. Returns null when the envelope has no `data` field
 * (or an empty one). Throws if [decrypted] is not the expected JSON (callers
 * already wrap the call in their own try/catch, matching the previous inline
 * `JSONObject(...)` behavior).
 *
 * Consolidates the identical parse that PhotoRepository, TrashViewModel,
 * SecureGalleryViewModel and PhotoViewerViewModel each inlined.
 *
 * NB: the thumbnail `data` is a single NO_WRAP base64 string. This is NOT the
 * newline-wrapped full-media value that must be stream-decoded via
 * Base64InputStream (see PhotoRepository's media path / ChunkedBlob) — do not
 * route those through here.
 */
fun decodeThumbEnvelope(decrypted: ByteArray): ByteArray? {
    val dataB64 = JSONObject(String(decrypted, Charsets.UTF_8)).optString("data", "")
    if (dataB64.isEmpty()) return null
    return Base64.decode(dataB64, Base64.NO_WRAP)
}
