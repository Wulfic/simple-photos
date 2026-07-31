/**
 * Compose Navigation host defining all screen routes and their arguments.
 */
package com.simplephotos.ui.navigation

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.hilt.navigation.compose.hiltViewModel
import com.simplephotos.data.album.VIEWER_PHOTO_IDS_KEY
import com.simplephotos.ui.components.HeaderNavigation
import com.simplephotos.ui.theme.ThemeState
import androidx.navigation.NavController
import androidx.navigation.NavType
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
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
 *
 * @param startRoute Optional deep-link for a window opened by
 *   [openInNewWindow] (#21). Already validated by [startRouteFromIntent]; it is
 *   pushed on top of the gallery (so Back still returns there) and only once
 *   the user is actually past login.
 */
@Composable
fun NavGraph(startRoute: String? = null) {
    val navController = rememberNavController()
    val viewModel: NavViewModel = hiltViewModel()
    val startDestination by viewModel.startDestination.collectAsState()
    val isAdmin by viewModel.isAdmin.collectAsState()

    if (startDestination == null) return // Loading

    // Bounded back-history cap (#37). Registered *before* NavHost so any
    // screen-level BackHandler (registered later, deeper in the tree) wins over
    // it via the dispatcher's LIFO ordering. It only fires when the stack is
    // more than [MAX_BACK_HISTORY] screens above the Gallery and the current
    // screen doesn't own its own Back — otherwise it's disabled and Back pops
    // normally, walking the genuine history one screen at a time.
    val backStack by navController.currentBackStack.collectAsState()
    val currentRoute = navController.currentBackStackEntryAsState().value?.destination?.route
    val capBack = shouldJumpToGalleryOnBack(backStack.map { it.destination.route }, currentRoute)
    BackHandler(enabled = capBack) { navController.navigateHome() }

    // Shared navbar state (#35) — one HeaderNavigation reused by every album /
    // library detail screen so the main navbar stays visible there, provided via
    // [LocalAppNav] instead of threading eight callbacks through each route.
    val username by viewModel.username.collectAsState()
    val isSystemDark = isSystemInDarkTheme()
    val appNav = remember(isAdmin, username, isSystemDark) {
        AppNavState(
            username = username,
            navigation = HeaderNavigation(
                onGalleryClick = { navController.navigateHome() },
                onAlbumsClick = { navController.navigateTopLevel(Screen.AlbumList.route) },
                onSearchClick = { navController.navigateTopLevel(Screen.Search.route) },
                onTrashClick = { navController.navigateTopLevel(Screen.Trash.route) },
                onSettingsClick = { navController.navigateTopLevel(Screen.Settings.route) },
                onSecureGalleryClick = { navController.navigateTopLevel(Screen.SecureGallery.route) },
                onSharedAlbumsClick = { navController.navigateTopLevel(Screen.SharedAlbums.route) },
                onDiagnosticsClick = { navController.navigateTopLevel(Screen.Diagnostics.route) },
                onLogout = { viewModel.logout { navController.navigate(Screen.Login.route) { popUpTo(0) } } },
                onToggleTheme = { ThemeState.toggle(viewModel.dataStore, ThemeState.isDark(isSystemDark)) },
                isAdmin = isAdmin,
            ),
        )
    }

    CompositionLocalProvider(LocalAppNav provides appNav) {
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
                onLoginSuccess = { navController.navigateHome() },
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
                onAlbumsClick = { navController.navigateTopLevel(Screen.AlbumList.route) },
                onSearchClick = { navController.navigateTopLevel(Screen.Search.route) },
                onTrashClick = { navController.navigateTopLevel(Screen.Trash.route) },
                onSettingsClick = { navController.navigateTopLevel(Screen.Settings.route) },
                onSecureGalleryClick = { navController.navigateTopLevel(Screen.SecureGallery.route) },
                onSharedAlbumsClick = { navController.navigateTopLevel(Screen.SharedAlbums.route) },
                onDiagnosticsClick = { navController.navigateTopLevel(Screen.Diagnostics.route) },
                onLogout = { navController.navigate(Screen.Login.route) { popUpTo(0) } },
                isAdmin = isAdmin
            )
        }
        composable(Screen.AlbumList.route) {
            AlbumListScreen(
                onGalleryClick = { navController.navigateHome() },
                onSearchClick = { navController.navigateTopLevel(Screen.Search.route) },
                onTrashClick = { navController.navigateTopLevel(Screen.Trash.route) },
                onSettingsClick = { navController.navigateTopLevel(Screen.Settings.route) },
                onSecureGalleryClick = { navController.navigateTopLevel(Screen.SecureGallery.route) },
                onSharedAlbumsClick = { navController.navigateTopLevel(Screen.SharedAlbums.route) },
                onDiagnosticsClick = { navController.navigateTopLevel(Screen.Diagnostics.route) },
                onLogout = { navController.navigate(Screen.Login.route) { popUpTo(0) } },
                onAlbumClick = { albumId -> navController.navigate(Screen.AlbumDetail.createRoute(albumId)) },
                onSharedAlbumClick = { navController.navigateTopLevel(Screen.SharedAlbums.route) },
                onPeople = { navController.navigate(Screen.People.base) },
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
                onGalleryClick = { navController.navigateHome() },
                onAlbumsClick = { navController.navigateTopLevel(Screen.AlbumList.route) },
                onSearchClick = { navController.navigateTopLevel(Screen.Search.route) },
                onSettingsClick = { navController.navigateTopLevel(Screen.Settings.route) },
                onSecureGalleryClick = { navController.navigateTopLevel(Screen.SecureGallery.route) },
                onSharedAlbumsClick = { navController.navigateTopLevel(Screen.SharedAlbums.route) },
                onDiagnosticsClick = { navController.navigateTopLevel(Screen.Diagnostics.route) },
                onLogout = { navController.navigate(Screen.Login.route) { popUpTo(0) } },
                isAdmin = isAdmin
            )
        }
        composable(Screen.Search.route) {
            SearchScreen(
                onPhotoClick = { photoId, photoIds -> navController.navigateToViewer(photoId, photoIds) },
                onGalleryClick = { navController.navigateHome() },
                onAlbumsClick = { navController.navigateTopLevel(Screen.AlbumList.route) },
                onTrashClick = { navController.navigateTopLevel(Screen.Trash.route) },
                onSettingsClick = { navController.navigateTopLevel(Screen.Settings.route) },
                onSecureGalleryClick = { navController.navigateTopLevel(Screen.SecureGallery.route) },
                onSharedAlbumsClick = { navController.navigateTopLevel(Screen.SharedAlbums.route) },
                onDiagnosticsClick = { navController.navigateTopLevel(Screen.Diagnostics.route) },
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
        ) { backStackEntry ->
            // Carry the launching grid's resolved order into this entry's saved
            // state BEFORE PhotoViewerScreen calls hiltViewModel(), so the
            // ViewModel's own SavedStateHandle already holds it at construction
            // (#52, E3a). This is a deliberate composition-time side effect:
            // doing it in a LaunchedEffect would run it *after* the ViewModel is
            // created, and the ViewModel's `init` has already started the
            // resolve by then. The `contains` guard makes it one-shot, so
            // recomposition — and a config change or process death, after which
            // the key is restored on this entry — cannot repeat it.
            //
            // Only the five non-album grids write the key; the gallery and album
            // detail leave it absent and the ViewModel falls back to the
            // resolver's own derivation, which for those two already matches
            // their grid element-for-element (E3).
            if (!backStackEntry.savedStateHandle.contains(VIEWER_PHOTO_IDS_KEY)) {
                navController.previousBackStackEntry
                    ?.savedStateHandle
                    ?.get<ArrayList<String>>(VIEWER_PHOTO_IDS_KEY)
                    ?.let { backStackEntry.savedStateHandle[VIEWER_PHOTO_IDS_KEY] = it }
            }
            PhotoViewerScreen(
                onBack = { navController.popBackStack() },
                onSelectPerson = { photoId, detectionId ->
                    navController.navigate(Screen.People.createAssignRoute(photoId, detectionId))
                },
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
        composable(
            route = Screen.People.route,
            arguments = listOf(
                navArgument("assignPhoto") { type = NavType.StringType; nullable = true; defaultValue = null },
                navArgument("assignDetection") { type = NavType.StringType; nullable = true; defaultValue = null },
            ),
        ) { backStackEntry ->
            val assignDetection = backStackEntry.arguments?.getString("assignDetection")?.toLongOrNull()
            PeopleScreen(
                onBack = { navController.popBackStack() },
                onPersonClick = { id -> navController.navigate(Screen.PersonDetail.createRoute(id)) },
                assignDetectionId = assignDetection,
                onAssigned = { navController.popBackStack() },
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
                onPhotoClick = { photoId, photoIds -> navController.navigateToViewer(photoId, photoIds) },
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
                onPhotoClick = { photoId, photoIds -> navController.navigateToViewer(photoId, photoIds) },
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
                onPhotoClick = { photoId, photoIds -> navController.navigateToViewer(photoId, photoIds) },
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
                onPhotoClick = { photoId, photoIds -> navController.navigateToViewer(photoId, photoIds) },
            )
        }
    }
    } // end CompositionLocalProvider(LocalAppNav)

    // A second window's deep-link (#21). Pushed on top of the resolved start
    // destination rather than replacing it, so Back leaves the user in the
    // gallery instead of closing the window. Gated on the gallery being the
    // start destination: if the user isn't logged in (or the server isn't set
    // up), the extra is dropped rather than navigated past the auth screens.
    LaunchedEffect(startRoute, startDestination) {
        if (startRoute != null && startDestination == Screen.Gallery.route) {
            navController.navigate(startRoute)
        }
    }
}

/**
 * Navigate to a top-level (drawer) destination without stacking a duplicate on
 * top of itself. Part of the #37 back-history fix: `launchSingleTop` keeps an
 * A→A re-tap from growing the stack; the [MAX_BACK_HISTORY] cap handles the
 * A↔B ping-pong case.
 */
private fun NavController.navigateTopLevel(route: String) =
    navigate(route) { launchSingleTop = true }

/**
 * Open the viewer on [photoId], handing it [photoIds] — the launching grid's own
 * resolved order (#52, E3a).
 *
 * For the five grids that resolve from *server* endpoints (Search, People, Pets,
 * Memories, Trips). Their order is relevance ranking / cluster order / curation
 * order, none of which the viewer can rebuild, so before this they paged the
 * gallery's `takenAt DESC` instead of what the user was looking at.
 *
 * The list rides on the *current* entry's `SavedStateHandle` — nav arguments
 * cannot carry a list, and `currentBackStackEntry` is the launching screen at the
 * moment of the call, which is exactly the `previousBackStackEntry` the viewer's
 * `composable` reads it back from. `ArrayList` rather than `List` because saved
 * state must be `Serializable` to survive process death.
 *
 * The gallery and album detail deliberately do NOT use this: [AlbumPhotoResolver]
 * already rebuilds their grids' lists exactly (E3), and handing one over as well
 * would give those two surfaces a second path to the same list — the drift this
 * whole workstream exists to remove.
 */
private fun NavController.navigateToViewer(photoId: String, photoIds: List<String>) {
    currentBackStackEntry?.savedStateHandle?.set(VIEWER_PHOTO_IDS_KEY, ArrayList(photoIds))
    navigate(Screen.PhotoViewer.createRoute(photoId))
}

/**
 * Reset to the Gallery home, clearing the back stack. Used by every "go home"
 * affordance and by the back-history cap ([shouldJumpToGalleryOnBack]).
 */
private fun NavController.navigateHome() =
    navigate(Screen.Gallery.route) {
        popUpTo(0)
        launchSingleTop = true
    }
