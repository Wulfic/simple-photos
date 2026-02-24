package com.simplephotos.ui.navigation

import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.navArgument
import com.simplephotos.ui.screens.album.AlbumDetailScreen
import com.simplephotos.ui.screens.album.AlbumListScreen
import com.simplephotos.ui.screens.auth.LoginScreen
import com.simplephotos.ui.screens.auth.RegisterScreen
import com.simplephotos.ui.screens.gallery.GalleryScreen
import com.simplephotos.ui.screens.settings.FolderSelectionScreen
import com.simplephotos.ui.screens.settings.SettingsScreen
import com.simplephotos.ui.screens.setup.ServerSetupScreen
import com.simplephotos.ui.screens.twofactor.TwoFactorSetupScreen
import com.simplephotos.ui.screens.viewer.PhotoViewerScreen

@Composable
fun NavGraph() {
    val navController = rememberNavController()
    val viewModel: NavViewModel = hiltViewModel()
    val startDestination by viewModel.startDestination.collectAsState()

    if (startDestination == null) return // Loading

    NavHost(
        navController = navController,
        startDestination = startDestination!!
    ) {
        composable(Screen.ServerSetup.route) {
            ServerSetupScreen(
                onSetupComplete = { navController.navigate(Screen.Login.route) { popUpTo(0) } }
            )
        }
        composable(Screen.Login.route) {
            LoginScreen(
                onLoginSuccess = { navController.navigate(Screen.Gallery.route) { popUpTo(0) } },
                onNavigateToRegister = { navController.navigate(Screen.Register.route) }
            )
        }
        composable(Screen.Register.route) {
            RegisterScreen(
                onRegisterSuccess = { navController.navigate(Screen.Login.route) { popUpTo(0) } },
                onNavigateToLogin = { navController.popBackStack() }
            )
        }
        composable(Screen.Gallery.route) {
            GalleryScreen(
                onPhotoClick = { photoId -> navController.navigate(Screen.PhotoViewer.createRoute(photoId)) },
                onAlbumsClick = { navController.navigate(Screen.AlbumList.route) },
                onSettingsClick = { navController.navigate(Screen.Settings.route) }
            )
        }
        composable(Screen.AlbumList.route) {
            AlbumListScreen(
                onBack = { navController.popBackStack() },
                onAlbumClick = { albumId -> navController.navigate(Screen.AlbumDetail.createRoute(albumId)) }
            )
        }
        composable(
            route = Screen.AlbumDetail.route,
            arguments = listOf(navArgument("albumId") { type = NavType.StringType })
        ) {
            AlbumDetailScreen(
                onBack = { navController.popBackStack() }
            )
        }
        composable(
            route = Screen.PhotoViewer.route,
            arguments = listOf(navArgument("photoId") { type = NavType.StringType })
        ) {
            PhotoViewerScreen(
                onBack = { navController.popBackStack() }
            )
        }
        composable(Screen.Settings.route) {
            SettingsScreen(
                onBack = { navController.popBackStack() },
                onLogout = { navController.navigate(Screen.Login.route) { popUpTo(0) } },
                onSetup2fa = { navController.navigate(Screen.TwoFactorSetup.route) },
                onBackupFolders = { navController.navigate(Screen.FolderSelection.route) }
            )
        }
        composable(Screen.TwoFactorSetup.route) {
            TwoFactorSetupScreen(
                onBack = { navController.popBackStack() }
            )
        }
        composable(Screen.FolderSelection.route) {
            FolderSelectionScreen(
                onBack = { navController.popBackStack() }
            )
        }
    }
}
