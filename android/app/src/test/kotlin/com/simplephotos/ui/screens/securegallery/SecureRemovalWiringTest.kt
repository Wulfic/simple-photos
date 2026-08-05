package com.simplephotos.ui.screens.securegallery

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Guards that the Z1 helpers are actually **called**, and that the sentences
 * they replaced are gone.
 *
 * This exists because of exactly how Z1 shipped on web: `56f995c` wrote
 * `secureRemovalPrompt`, unit-tested it fully, and wired it into nothing, while
 * the component kept a raw prompt containing the very sentence the helper was
 * written to kill. Its twin *was* wired, which is what made the omission read as
 * done. **A tested helper with no call site is worse than no helper** — the
 * green suite is what stops anyone looking.
 *
 * Compose UI cannot be asserted here (that needs a device, which is the whole
 * reason this class of bug survives on Android longer than on web), so what is
 * checkable is the source: the call site exists, and the false claims do not.
 * Reading source in a test follows web's `safeArea.test.ts` precedent.
 */
class SecureRemovalWiringTest {

    /**
     * Module source root. Gradle runs unit tests with the module directory as
     * the working directory, but walking up makes this survive being run from
     * the repo root too — and a wrong guess would make every assertion below
     * pass vacuously, which is the failure this whole file is about.
     */
    private fun source(relative: String): String {
        var dir: File? = File(System.getProperty("user.dir")).absoluteFile
        while (dir != null) {
            val candidate = File(dir, "app/src/main/kotlin/$relative")
            if (candidate.isFile) return candidate.readText()
            val here = File(dir, "src/main/kotlin/$relative")
            if (here.isFile) return here.readText()
            dir = dir.parentFile
        }
        throw AssertionError(
            "could not locate $relative from ${System.getProperty("user.dir")} — " +
                "this test cannot pass without reading it"
        )
    }

    private val galleryDetailView =
        source("com/simplephotos/ui/screens/securegallery/GalleryDetailView.kt")
    private val secureViewModel =
        source("com/simplephotos/ui/screens/securegallery/SecureGalleryViewModel.kt")
    private val securePhotoViewer =
        source("com/simplephotos/ui/screens/securegallery/SecurePhotoViewer.kt")
    private val albumDetailScreen =
        source("com/simplephotos/ui/screens/album/AlbumDetailScreen.kt")

    @Test
    fun `the secure removal prompt has a call site`() {
        assertTrue(
            "GalleryDetailView must ASK AlbumRemoval what to say — a helper nobody " +
                "calls is how Z1 shipped half-wired the first time",
            galleryDetailView.contains("AlbumRemoval.secureRemovalPrompt("),
        )
        assertTrue(
            "and must resolve membership rather than assuming it",
            galleryDetailView.contains("AlbumRemoval.otherSecureAlbumCount("),
        )
    }

    @Test
    fun `the blocked verdict is rendered, not just returned`() {
        // A `when` that handles only Confirm would not compile, but a call site
        // could still drop the refusal by never offering the recovery.
        assertTrue(
            "the refusal must offer the refresh that can resolve it",
            galleryDetailView.contains("SecureRemovalVerdict.Blocked") &&
                galleryDetailView.contains("refreshFeeds()"),
        )
    }

    @Test
    fun `the confirmation counts the same batch the removal will act on`() {
        assertTrue(
            "the prompt must expand bursts through the shared planner, or it " +
                "describes a different set than the one being removed",
            galleryDetailView.contains("SecureMovePlan.expandForRemoval("),
        )
        assertTrue(
            "and so must the removal itself",
            secureViewModel.contains("SecureMovePlan.expandForRemoval("),
        )
    }

    @Test
    fun `pushing to another secure album ADDS instead of moving`() {
        // The reported Z1 bug, in the one place it lived on Android.
        val start = secureViewModel.indexOf("fun pushItemsTo(")
        assertTrue("pushItemsTo must exist", start > 0)
        val end = secureViewModel.indexOf("\n    /**", start)
        assertTrue("pushItemsTo must be followed by another member", end > start)
        val body = secureViewModel.substring(start, end)

        assertTrue(
            "pushItemsTo must call addItem — a '+' that moves is the reported bug",
            body.contains("secureGalleryRepository.addItem("),
        )
        assertFalse(
            "pushItemsTo must NOT call moveItem: adding a secure photo to a " +
                "second album emptied it out of the first",
            body.contains("moveItem("),
        )
        assertTrue(
            "a 409 is 'already in that album', not a failure",
            body.contains("isConflict("),
        )
    }

    @Test
    fun `the pull picker still moves`() {
        // Z1 removed the constraint that forced every transfer to be a move; it
        // did not make every transfer a copy. "Bring these here" is still a move.
        val start = secureViewModel.indexOf("fun moveItemsIntoSelected(")
        assertTrue("moveItemsIntoSelected must exist", start > 0)
        val end = secureViewModel.indexOf("\n    // ──", start)
        assertTrue("moveItemsIntoSelected must be followed by a section break", end > start)
        assertTrue(
            "the #31 pull picker moves by design",
            secureViewModel.substring(start, end).contains("secureGalleryRepository.moveItem("),
        )
    }

    @Test
    fun `no screen still promises the photo returns to the regular gallery`() {
        // The exact literals Z1e deleted. Matching literals rather than the
        // phrase keeps the doc comments — which discuss the false sentence at
        // length, on purpose — from tripping this.
        val lies = listOf(
            "\"The selected photos will return to your regular gallery.\"",
            "\"The photo will return to your regular gallery.\"",
            "\"This burst (all of its frames) will return to your regular gallery.\"",
        )
        for (file in listOf(galleryDetailView, securePhotoViewer)) {
            for (lie in lies) {
                assertFalse("a screen still states $lie unconditionally", file.contains(lie))
            }
        }
    }

    @Test
    fun `the regular album header confirms and offers an add`() {
        // The other half of the reported divergence: a Close icon, no
        // confirmation at all, and no way to file a selection elsewhere.
        assertTrue(
            "removing from an ordinary album must confirm",
            albumDetailScreen.contains("AlbumRemoval.albumRemovalPrompt("),
        )
        assertTrue(
            "and the header must offer the add the report asked for",
            albumDetailScreen.contains("AlbumPickerDialog("),
        )
    }
}
