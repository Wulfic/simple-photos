/**
 * Room type converters for column types SQLite can't store natively.
 */
package com.simplephotos.data.local

import androidx.room.TypeConverter
import com.simplephotos.data.media.Rendition
import org.json.JSONArray
import org.json.JSONObject

/**
 * Converters registered on [AppDatabase].
 *
 * JSON rather than a delimiter-joined string: blob ids are opaque server-issued
 * strings, so any separator we picked would be an assumption about their
 * alphabet, and a single id containing it would silently split one member into
 * two — corrupting album membership in a way nothing downstream could detect.
 */
class Converters {
    @TypeConverter
    fun stringListToJson(value: List<String>): String = JSONArray(value).toString()

    @TypeConverter
    fun jsonToStringList(value: String): List<String> {
        if (value.isBlank()) return emptyList()
        return try {
            val arr = JSONArray(value)
            List(arr.length()) { arr.getString(it) }
        } catch (e: Exception) {
            // A row we can't parse must not take the whole album list down with
            // it — an empty membership re-derives from the server manifest on the
            // next sync, which is the recoverable outcome.
            android.util.Log.w("Converters", "could not parse stored string list: ${e.message}")
            emptyList()
        }
    }

    // ── Video resolution ladder (#49) ────────────────────────────────────────
    // Stored as JSON on the photo row rather than in a table of its own: a
    // ladder is read only with the photo that owns it, is at most three entries,
    // and is replaced wholesale by every sync. A join table would buy nothing
    // and would need its own cascade.

    @TypeConverter
    fun renditionListToJson(value: List<Rendition>): String {
        val arr = JSONArray()
        value.forEach { r ->
            arr.put(
                JSONObject().apply {
                    put("shortEdge", r.shortEdge)
                    put("width", r.width)
                    put("height", r.height)
                    put("isSource", r.isSource)
                    // put(key, null) REMOVES the key, which is what we want:
                    // absent and null read back identically below.
                    put("blobId", r.blobId)
                    put("codec", r.codec)
                    put("sizeBytes", r.sizeBytes)
                }
            )
        }
        return arr.toString()
    }

    @TypeConverter
    fun jsonToRenditionList(value: String): List<Rendition> {
        if (value.isBlank()) return emptyList()
        return try {
            val arr = JSONArray(value)
            List(arr.length()) { i ->
                val o = arr.getJSONObject(i)
                Rendition(
                    shortEdge = o.getInt("shortEdge"),
                    width = o.getInt("width"),
                    height = o.getInt("height"),
                    isSource = o.optBoolean("isSource", false),
                    // NOT optString: it returns "" for an absent key, and an
                    // empty blob id is not null, so it would survive the
                    // picker's `blobId != null` filter and then build a
                    // hostless `spblob://` URI that fails at playback. The
                    // difference only shows on an unencrypted install, where
                    // every rung has a null blob id.
                    blobId = if (o.isNull("blobId")) null else o.getString("blobId"),
                    codec = if (o.isNull("codec")) null else o.getString("codec"),
                    sizeBytes = o.optLong("sizeBytes", 0L),
                )
            }
        } catch (e: Exception) {
            // A row we can't parse must not take the gallery down with it. An
            // empty ladder means "one quality, no picker" — the pre-#49
            // behaviour — and the next sync rewrites it from the server.
            android.util.Log.w("Converters", "could not parse stored rendition list: ${e.message}")
            emptyList()
        }
    }
}
