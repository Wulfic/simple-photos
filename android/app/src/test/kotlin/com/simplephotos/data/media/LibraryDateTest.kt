package com.simplephotos.data.media

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * P1 (owner, 2026-11-26): downloaded photos sorted at the bottom. Measured on the
 * live library: 5,003 of 5,146 Android-uploaded rows had `taken_at = 1970-01-01`
 * because `MediaStore.DATE_TAKEN` is NULL for a downloaded file and the scanner
 * stored `getLong()` = 0. Rule: camera tree keeps its capture date; everything
 * else sorts by the date it landed on the phone.
 */
class LibraryDateTest {

    private val now = 1_795_000_000_000L          // 2026-11-17, arbitrary "now"
    private val landed = 1_754_400_000L           // 2025-08-05 (seconds, MediaStore unit)
    private val exif = 1_400_000_000_000L         // 2014-05-13 (ms)

    private fun date(
        path: String? = "Download/",
        taken: Long? = null,
        added: Long? = landed,
        modified: Long? = landed,
    ) = LibraryDate.forScan(path, taken, added, modified, now)

    // ── The bug ──────────────────────────────────────────────────────────

    @Test fun `dateless download sorts by the day it landed, not 1970`() {
        assertEquals(landed * 1000, date(path = "Download/", taken = null))
    }

    @Test fun `result is never zero or negative even with every column empty`() {
        val d = date(path = "Download/", taken = null, added = null, modified = null)
        assertEquals(now, d)
    }

    // ── Camera tree ──────────────────────────────────────────────────────

    @Test fun `camera photo keeps its capture date`() {
        assertEquals(exif, date(path = "DCIM/Camera/", taken = exif))
    }

    @Test fun `camera photo with no capture date falls back to the landed date`() {
        assertEquals(landed * 1000, date(path = "DCIM/Camera/", taken = null))
    }

    @Test fun `camera photo with a zero capture date is not treated as 1970`() {
        assertEquals(landed * 1000, date(path = "DCIM/Camera/", taken = 0L))
    }

    // ── Outside the camera: landed date wins even when EXIF exists ───────

    @Test fun `download carrying an old EXIF date still sorts by the landed date`() {
        assertEquals(landed * 1000, date(path = "Download/", taken = exif))
    }

    @Test fun `messaging-app image sorts by the landed date`() {
        assertEquals(
            landed * 1000,
            date(path = "Android/media/com.whatsapp/WhatsApp/Media/WhatsApp Images/", taken = exif),
        )
    }

    // ── Earlier of DATE_ADDED / DATE_MODIFIED ────────────────────────────

    @Test fun `phone migration resets date added but keeps mtime - the earlier wins`() {
        val migratedAt = landed + 86_400 * 300
        assertEquals(landed * 1000, date(added = migratedAt, modified = landed))
    }

    @Test fun `an edit bumps mtime but the landed date stays`() {
        assertEquals(landed * 1000, date(added = landed, modified = landed + 86_400 * 30))
    }

    @Test fun `a pre-Android mtime (zip FAT 1980 stamp) is ignored`() {
        val fat1980 = 315_532_800L
        assertEquals(landed * 1000, date(added = landed, modified = fat1980))
    }

    @Test fun `non-positive added and modified are ignored`() {
        assertEquals(landed * 1000, date(added = 0L, modified = landed))
        assertEquals(landed * 1000, date(added = landed, modified = -1L))
    }

    @Test fun `outside the camera with no usable landed date the capture date is used`() {
        assertEquals(exif, date(path = "Download/", taken = exif, added = 0L, modified = null))
    }

    // ── Camera-tree detection ────────────────────────────────────────────

    @Test fun `camera tree is DCIM as the first path segment, any case`() {
        assertTrue(LibraryDate.isCameraTree("DCIM/Camera/"))
        assertTrue(LibraryDate.isCameraTree("DCIM/Screenshots/"))
        assertTrue(LibraryDate.isCameraTree("dcim/Camera"))
        assertTrue(LibraryDate.isCameraTree("/DCIM/"))
        assertFalse(LibraryDate.isCameraTree("Pictures/DCIM/"))
        assertFalse(LibraryDate.isCameraTree("Download/"))
        assertFalse(LibraryDate.isCameraTree("DCIMx/"))
        assertFalse(LibraryDate.isCameraTree(""))
        assertFalse(LibraryDate.isCameraTree(null))
    }

    @Test fun `absolute path (API below 29) maps to a relative path from DCIM`() {
        assertEquals("DCIM/Camera/a.jpg",
            LibraryDate.relativeFromAbsolute("/storage/emulated/0/DCIM/Camera/a.jpg"))
        assertEquals("DCIM/Camera/a.jpg",
            LibraryDate.relativeFromAbsolute("/storage/1234-ABCD/DCIM/Camera/a.jpg"))
        assertFalse(LibraryDate.isCameraTree(
            LibraryDate.relativeFromAbsolute("/storage/emulated/0/Download/a.jpg")))
        assertEquals(null, LibraryDate.relativeFromAbsolute(null))
    }
}
