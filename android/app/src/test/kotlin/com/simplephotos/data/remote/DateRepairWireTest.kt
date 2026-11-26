package com.simplephotos.data.remote

import com.google.gson.Gson
import com.google.gson.JsonParser
import com.simplephotos.data.remote.dto.DateRepairItem
import com.simplephotos.data.remote.dto.DateRepairRequest
import com.simplephotos.data.remote.dto.DateRepairResponse
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * P1c wire format. The server refuses an ABSENT `expect_taken_at` (no claim) but
 * accepts an explicit `null` ("the column is NULL"). The app's Retrofit Gson is a
 * default `Gson()` — it OMITS null fields — so without the item's own adapter a
 * NULL claim would turn into a 400 for its whole chunk of 500.
 */
class DateRepairWireTest {

    /** Same instance the app gets from `GsonConverterFactory.create()`. */
    private val gson = Gson()

    private fun json(req: DateRepairRequest) = JsonParser.parseString(gson.toJson(req)).asJsonObject

    @Test fun `a null expectation is sent as an explicit null, not dropped`() {
        val item = json(DateRepairRequest(true, listOf(DateRepairItem("p1", "2025-08-05T00:00:00Z", null))))
            .getAsJsonArray("items")[0].asJsonObject
        assertTrue("expect_taken_at must be present", item.has("expect_taken_at"))
        assertTrue(item.get("expect_taken_at").isJsonNull)
    }

    @Test fun `a string expectation and the snake_case names go out as the server reads them`() {
        val body = json(DateRepairRequest(false, listOf(DateRepairItem("p1", "2025-08-05T00:00:00Z", "1970-01-01T00:00:00.000Z"))))
        assertFalse(body.get("dry_run").asBoolean)
        val item = body.getAsJsonArray("items")[0].asJsonObject
        assertEquals("p1", item.get("photo_id").asString)
        assertEquals("2025-08-05T00:00:00Z", item.get("taken_at").asString)
        assertEquals("1970-01-01T00:00:00.000Z", item.get("expect_taken_at").asString)
    }

    @Test fun `the adapter does not leak serializeNulls to the rest of the request`() {
        // dry_run is never null, but a future nullable field on the request must
        // keep Gson's default (omit) — the adapter restores the writer's setting.
        val raw = gson.toJson(DateRepairRequest(true, listOf(DateRepairItem("p", "t", null))))
        assertEquals(1, Regex("null").findAll(raw).count())
    }

    @Test fun `a response with the protected count parses`() {
        val r = gson.fromJson(
            """{"dry_run":true,"batch_id":null,"counts":{"change":2,"unchanged":0,"stale":1,"not_found":0,"protected":3},
               "items":[{"photo_id":"p","outcome":"protected","current_taken_at":null,"new_taken_at":"x"}]}""",
            DateRepairResponse::class.java,
        )
        assertEquals(3, r.counts.protected)
        assertEquals(1, r.counts.stale)
        assertEquals("protected", r.items.single().outcome)
    }
}
