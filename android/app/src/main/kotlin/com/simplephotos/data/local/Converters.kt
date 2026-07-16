/**
 * Room type converters for column types SQLite can't store natively.
 */
package com.simplephotos.data.local

import androidx.room.TypeConverter
import org.json.JSONArray

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
}
