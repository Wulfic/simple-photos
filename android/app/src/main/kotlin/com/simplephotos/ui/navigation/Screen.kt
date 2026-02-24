package com.simplephotos.ui.navigation

sealed class Screen(val route: String) {
    data object ServerSetup : Screen("server_setup")
    data object Login : Screen("login")
    data object Register : Screen("register")
    data object Gallery : Screen("gallery")
    data object AlbumList : Screen("album_list")
    data object AlbumDetail : Screen("album_detail/{albumId}") {
        fun createRoute(albumId: String) = "album_detail/$albumId"
    }
    data object PhotoViewer : Screen("photo_viewer/{photoId}") {
        fun createRoute(photoId: String) = "photo_viewer/$photoId"
    }
    data object Settings : Screen("settings")
    data object TwoFactorSetup : Screen("two_factor_setup")
    data object FolderSelection : Screen("folder_selection")
}
