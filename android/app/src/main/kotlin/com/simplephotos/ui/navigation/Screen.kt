/**
 * Sealed hierarchy of navigation route definitions used by [NavGraph].
 */
package com.simplephotos.ui.navigation

/** Navigation route definitions for [NavGraph]. */
sealed class Screen(val route: String) {
    data object ServerSetup : Screen("server_setup")
    data object Login : Screen("login")
    data object Register : Screen("register")
    data object Gallery : Screen("gallery")
    data object AlbumList : Screen("album_list")
    data object AlbumDetail : Screen("album_detail/{albumId}") {
        fun createRoute(albumId: String) = "album_detail/$albumId"
    }
    data object PhotoViewer : Screen("photo_viewer/{photoId}?albumId={albumId}") {
        fun createRoute(photoId: String, albumId: String? = null): String {
            val base = "photo_viewer/$photoId"
            return if (albumId != null) "$base?albumId=$albumId" else base
        }
    }
    data object Settings : Screen("settings")
    data object Search : Screen("search")
    data object Trash : Screen("trash")
    data object TwoFactorSetup : Screen("two_factor_setup")
    data object FolderSelection : Screen("folder_selection")
    data object SecureGallery : Screen("secure_gallery")
    data object SharedAlbums : Screen("shared_albums")
    data object Diagnostics : Screen("diagnostics")
    // ── Smart album sub-pages (entered from Albums screen, not Library hub) ──
    data object People : Screen("library/people")
    data object Pets : Screen("library/pets")
    data object Memories : Screen("library/memories")
    data object Trips : Screen("library/trips")
    data object PersonDetail : Screen("library/people/{clusterId}") {
        fun createRoute(clusterId: Long) = "library/people/$clusterId"
    }
    data object PetDetail : Screen("library/pets/{clusterId}") {
        fun createRoute(clusterId: Long) = "library/pets/$clusterId"
    }
    data object MemoryDetail : Screen("library/memories/{memoryId}") {
        fun createRoute(memoryId: String) = "library/memories/$memoryId"
    }
    data object TripDetail : Screen("library/trips/{tripId}") {
        fun createRoute(tripId: String) = "library/trips/$tripId"
    }
}
