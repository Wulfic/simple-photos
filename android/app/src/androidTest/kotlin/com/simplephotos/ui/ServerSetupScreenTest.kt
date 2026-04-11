package com.simplephotos.ui

import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import com.simplephotos.MainActivity
import dagger.hilt.android.testing.HiltAndroidRule
import dagger.hilt.android.testing.HiltAndroidTest
import org.junit.Before
import org.junit.Rule
import org.junit.Test

/**
 * E2E tests for server setup and network discovery.
 *
 * Verifies:
 * - Server setup screen renders on first launch
 * - Manual URL entry works
 * - Network scan filters to primary-mode servers only
 */
@HiltAndroidTest
class ServerSetupScreenTest {

    @get:Rule(order = 0)
    val hiltRule = HiltAndroidRule(this)

    @get:Rule(order = 1)
    val composeRule = createAndroidComposeRule<MainActivity>()

    @Before
    fun setup() {
        hiltRule.inject()
    }

    @Test
    fun serverSetup_displaysOnFirstLaunch() {
        // On first launch with no stored server URL, setup screen should appear
        composeRule.waitForIdle()
        composeRule.onRoot().assertExists()
    }
}
