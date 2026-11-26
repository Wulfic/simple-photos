package com.simplephotos.data.media

import java.time.Instant
import java.time.LocalDateTime
import java.time.OffsetDateTime
import java.time.ZoneOffset

/**
 * The timeline date of a server photo record, in epoch ms (TODO §4b P1c).
 *
 * The rule is the server's own sort key, `COALESCE(taken_at, created_at)`, and
 * the web client's `recordTakenAt` — three copies of one rule, so none of them
 * "corrects" the rows another wrote on every pass.
 *
 * Pure (no `android.util.Log`, which throws in JVM tests).
 */
object ServerDate {

    /** Parse an ISO-8601 instant; naive timestamps are UTC. Null when unparseable. */
    fun parseMs(iso: String?): Long? {
        if (iso.isNullOrBlank()) return null
        val s = iso.trim()
        return runCatching { Instant.parse(s).toEpochMilli() }
            .recoverCatching { OffsetDateTime.parse(s).toInstant().toEpochMilli() }
            .recoverCatching { LocalDateTime.parse(s).toInstant(ZoneOffset.UTC).toEpochMilli() }
            .getOrNull()
    }

    /** `taken_at`, or `created_at` when `taken_at` is NULL. Null when unparseable. */
    fun ofRecord(takenAt: String?, createdAt: String?): Long? =
        parseMs(if (takenAt.isNullOrBlank()) createdAt else takenAt)

    /**
     * What an already-mirrored row's `takenAt` should become after a sync, or null
     * to leave it. Server wins (like crop and favourite) — a server-side date repair
     * reaches this device only through here. An unparseable server value never
     * overwrites a cached one.
     */
    fun adopt(existingMs: Long, takenAt: String?, createdAt: String?): Long? {
        val server = ofRecord(takenAt, createdAt) ?: return null
        return server.takeIf { it != existingMs }
    }
}
