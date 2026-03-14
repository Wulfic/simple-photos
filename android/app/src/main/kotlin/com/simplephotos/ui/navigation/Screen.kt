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
}
