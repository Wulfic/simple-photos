/**
 * Compose Navigation host defining all screen routes and their arguments.
 */
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
import com.simplephotos.ui.screens.trash.TrashScreen
import com.simplephotos.ui.screens.twofactor.TwoFactorSetupScreen
import com.simplephotos.ui.screens.viewer.PhotoViewerScreen
import com.simplephotos.ui.screens.search.SearchScreen
import com.simplephotos.ui.screens.diagnostics.DiagnosticsScreen
import com.simplephotos.ui.screens.securegallery.SecureGalleryScreen
import com.simplephotos.ui.screens.sharing.SharedAlbumsScreen
import com.simplephotos.ui.screens.library.PeopleScreen
import com.simplephotos.ui.screens.library.PetsScreen
import com.simplephotos.ui.screens.library.MemoriesScreen
import com.simplephotos.ui.screens.library.TripsScreen
import com.simplephotos.ui.screens.library.PersonDetailScreen
import com.simplephotos.ui.screens.library.PetDetailScreen
import com.simplephotos.ui.screens.library.MemoryDetailScreen
import com.simplephotos.ui.screens.library.TripDetailScreen

/**
 * Top-level navigation host. Routes are defined in [Screen].
 * The start destination is resolved at runtime by [NavViewModel]
 * (server setup → login → gallery).
 */
@Composable
fun NavGraph() {
    val navController = rememberNavController()
    val viewModel: NavViewModel = hiltViewModel()
    val startDestination by viewModel.startDestination.collectAsState()
    val isAdmin by viewModel.isAdmin.collectAsState()

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
                onSearchClick = { navController.navigate(Screen.Search.route) },
                onTrashClick = { navController.navigate(Screen.Trash.route) },
                onSettingsClick = { navController.navigate(Screen.Settings.route) },
                onSecureGalleryClick = { navController.navigate(Screen.SecureGallery.route) },
                onSharedAlbumsClick = { navController.navigate(Screen.SharedAlbums.route) },
                onDiagnosticsClick = { navController.navigate(Screen.Diagnostics.route) },
                onLogout = { navController.navigate(Screen.Login.route) { popUpTo(0) } },
                isAdmin = isAdmin
            )
        }
        composable(Screen.AlbumList.route) {
            AlbumListScreen(
                onGalleryClick = { navController.navigate(Screen.Gallery.route) { popUpTo(0) } },
                onSearchClick = { navController.navigate(Screen.Search.route) },
                onTrashClick = { navController.navigate(Screen.Trash.route) },
                onSettingsClick = { navController.navigate(Screen.Settings.route) },
                onSecureGalleryClick = { navController.navigate(Screen.SecureGallery.route) },
                onSharedAlbumsClick = { navController.navigate(Screen.SharedAlbums.route) },
                onDiagnosticsClick = { navController.navigate(Screen.Diagnostics.route) },
                onLogout = { navController.navigate(Screen.Login.route) { popUpTo(0) } },
                onAlbumClick = { albumId -> navController.navigate(Screen.AlbumDetail.createRoute(albumId)) },
                onSharedAlbumClick = { navController.navigate(Screen.SharedAlbums.route) },
                onPeople = { navController.navigate(Screen.People.route) },
                onPets = { navController.navigate(Screen.Pets.route) },
                onMemories = { navController.navigate(Screen.Memories.route) },
                onTrips = { navController.navigate(Screen.Trips.route) },
                onPersonClick = { id -> navController.navigate(Screen.PersonDetail.createRoute(id)) },
                onPetClick = { id -> navController.navigate(Screen.PetDetail.createRoute(id)) },
                onMemoryClick = { id -> navController.navigate(Screen.MemoryDetail.createRoute(id)) },
                onTripClick = { id -> navController.navigate(Screen.TripDetail.createRoute(id)) },
                isAdmin = isAdmin
            )
        }
        composable(Screen.Trash.route) {
            TrashScreen(
                onGalleryClick = { navController.navigate(Screen.Gallery.route) { popUpTo(0) } },
                onAlbumsClick = { navController.navigate(Screen.AlbumList.route) },
                onSearchClick = { navController.navigate(Screen.Search.route) },
                onSettingsClick = { navController.navigate(Screen.Settings.route) },
                onSecureGalleryClick = { navController.navigate(Screen.SecureGallery.route) },
                onSharedAlbumsClick = { navController.navigate(Screen.SharedAlbums.route) },
                onDiagnosticsClick = { navController.navigate(Screen.Diagnostics.route) },
                onLogout = { navController.navigate(Screen.Login.route) { popUpTo(0) } },
                isAdmin = isAdmin
            )
        }
        composable(Screen.Search.route) {
            SearchScreen(
                onPhotoClick = { photoId -> navController.navigate(Screen.PhotoViewer.createRoute(photoId)) },
                onGalleryClick = { navController.navigate(Screen.Gallery.route) { popUpTo(0) } },
                onAlbumsClick = { navController.navigate(Screen.AlbumList.route) },
                onTrashClick = { navController.navigate(Screen.Trash.route) },
                onSettingsClick = { navController.navigate(Screen.Settings.route) },
                onSecureGalleryClick = { navController.navigate(Screen.SecureGallery.route) },
                onSharedAlbumsClick = { navController.navigate(Screen.SharedAlbums.route) },
                onDiagnosticsClick = { navController.navigate(Screen.Diagnostics.route) },
                onLogout = { navController.navigate(Screen.Login.route) { popUpTo(0) } },
                isAdmin = isAdmin
            )
        }
        composable(
            route = Screen.AlbumDetail.route,
            arguments = listOf(navArgument("albumId") { type = NavType.StringType })
        ) { backStackEntry ->
            val albumId = backStackEntry.arguments?.getString("albumId")
            AlbumDetailScreen(
                onBack = { navController.popBackStack() },
                onPhotoClick = { photoId -> navController.navigate(Screen.PhotoViewer.createRoute(photoId, albumId)) }
            )
        }
        composable(
            route = Screen.PhotoViewer.route,
            arguments = listOf(
                navArgument("photoId") { type = NavType.StringType },
                navArgument("albumId") { type = NavType.StringType; nullable = true; defaultValue = null }
            )
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
        composable(Screen.Diagnostics.route) {
            DiagnosticsScreen(
                onBack = { navController.popBackStack() }
            )
        }
        composable(Screen.SecureGallery.route) {
            SecureGalleryScreen(
                onBack = { navController.popBackStack() }
            )
        }
        composable(Screen.SharedAlbums.route) {
            SharedAlbumsScreen(
                onBack = { navController.popBackStack() }
            )
        }
        composable(Screen.People.route) {
            PeopleScreen(
                onBack = { navController.popBackStack() },
                onPersonClick = { id -> navController.navigate(Screen.PersonDetail.createRoute(id)) },
            )
        }
        composable(Screen.Pets.route) {
            PetsScreen(
                onBack = { navController.popBackStack() },
                onPetClick = { id -> navController.navigate(Screen.PetDetail.createRoute(id)) },
            )
        }
        composable(Screen.Memories.route) {
            MemoriesScreen(
                onBack = { navController.popBackStack() },
                onMemoryClick = { id -> navController.navigate(Screen.MemoryDetail.createRoute(id)) },
            )
        }
        composable(Screen.Trips.route) {
            TripsScreen(
                onBack = { navController.popBackStack() },
                onTripClick = { id -> navController.navigate(Screen.TripDetail.createRoute(id)) },
            )
        }
        composable(
            route = Screen.PersonDetail.route,
            arguments = listOf(navArgument("clusterId") { type = NavType.LongType })
        ) { backStackEntry ->
            val clusterId = backStackEntry.arguments?.getLong("clusterId") ?: 0L
            PersonDetailScreen(
                clusterId = clusterId,
                onBack = { navController.popBackStack() },
                onPhotoClick = { photoId -> navController.navigate(Screen.PhotoViewer.createRoute(photoId)) },
            )
        }
        composable(
            route = Screen.PetDetail.route,
            arguments = listOf(navArgument("clusterId") { type = NavType.LongType })
        ) { backStackEntry ->
            val clusterId = backStackEntry.arguments?.getLong("clusterId") ?: 0L
            PetDetailScreen(
                clusterId = clusterId,
                onBack = { navController.popBackStack() },
                onPhotoClick = { photoId -> navController.navigate(Screen.PhotoViewer.createRoute(photoId)) },
            )
        }
        composable(
            route = Screen.MemoryDetail.route,
            arguments = listOf(navArgument("memoryId") { type = NavType.StringType })
        ) { backStackEntry ->
            val memoryId = backStackEntry.arguments?.getString("memoryId") ?: ""
            MemoryDetailScreen(
                memoryId = memoryId,
                onBack = { navController.popBackStack() },
                onPhotoClick = { photoId -> navController.navigate(Screen.PhotoViewer.createRoute(photoId)) },
            )
        }
        composable(
            route = Screen.TripDetail.route,
            arguments = listOf(navArgument("tripId") { type = NavType.StringType })
        ) { backStackEntry ->
            val tripId = backStackEntry.arguments?.getString("tripId") ?: ""
            TripDetailScreen(
                tripId = tripId,
                onBack = { navController.popBackStack() },
                onPhotoClick = { photoId -> navController.navigate(Screen.PhotoViewer.createRoute(photoId)) },
            )
        }
    }
}
