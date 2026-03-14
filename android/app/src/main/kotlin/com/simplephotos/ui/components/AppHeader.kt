package com.simplephotos.ui.components

import androidx.compose.animation.core.*
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.SweepGradient
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.rotate
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.DpOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.simplephotos.R
import com.simplephotos.ui.theme.ThemeState

/**
 * Which main tab is currently active.
 */
enum class ActiveTab { GALLERY, ALBUMS, SEARCH, TRASH }

/**
 * Navigation callback bundle passed into [AppHeader].
 */
data class HeaderNavigation(
    val onGalleryClick: () -> Unit = {},
    val onAlbumsClick: () -> Unit = {},
    val onSearchClick: () -> Unit = {},
    val onTrashClick: () -> Unit = {},
    val onSettingsClick: () -> Unit = {},
    val onSecureGalleryClick: () -> Unit = {},
    val onSharedAlbumsClick: () -> Unit = {},
    val onDiagnosticsClick: () -> Unit = {},
    val onLogout: () -> Unit = {},
    val onToggleTheme: () -> Unit = {},
    val isAdmin: Boolean = false,
)

/**
 * Shared application header that mirrors the web UI.
 *
 * Layout (left → right):
 *  Logo + "Simple Photos" | Gallery | Albums | Trash | [children] | spacer | theme toggle | avatar + dropdown
 *
 * @param activeTab      Which tab to highlight as active.
 * @param username       Logged-in username (shown in avatar + dropdown).
 * @param navigation     Navigation callbacks.
 * @param isSyncing      Whether background processing is active (shows pulse indicator).
 * @param syncLabel      Optional label for the sync activity.
 * @param children       Slot for page-specific action buttons (e.g. Upload, Sync).
 */
@Composable
fun AppHeader(
    activeTab: ActiveTab,
    username: String,
    navigation: HeaderNavigation,
    isSyncing: Boolean = false,
    syncLabel: String? = null,
    children: @Composable RowScope.() -> Unit = {},
) {
    // ── Theme-aware colors ───────────────────────────────────────
    val isLightTheme = MaterialTheme.colorScheme.background.luminance() > 0.5f

    val headerGradient = if (isLightTheme) {
        Brush.horizontalGradient(
            colors = listOf(
                Color(0xFFF8FAFC), // slate-50
                Color(0xFFEFF6FF), // blue-50
                Color(0xFFF8FAFC), // slate-50
            )
        )
    } else {
        Brush.horizontalGradient(
            colors = listOf(
                Color(0xFF111827), // gray-900
                Color(0xFF1F2937), // gray-800
                Color(0xFF111827), // gray-900
            )
        )
    }
    val borderColor = if (isLightTheme) Color.Black.copy(alpha = 0.08f) else Color.White.copy(alpha = 0.1f)
    val inactiveTextColor = if (isLightTheme) Color(0xFF6B7280) else Color(0xFF9CA3AF) // gray-500 / gray-400
    val activeTabBg = if (isLightTheme) MaterialTheme.colorScheme.primary.copy(alpha = 0.12f) else Color.White.copy(alpha = 0.15f)
    val activeTabText = if (isLightTheme) MaterialTheme.colorScheme.primary else Color.White

    Surface(
        shadowElevation = 4.dp,
        color = Color.Transparent
    ) {
        Column(
            modifier = Modifier.background(headerGradient)
        ) {
            // Status bar spacer — pushes content below system icons / camera notch
            Spacer(
                modifier = Modifier
                    .fillMaxWidth()
                    .windowInsetsPadding(WindowInsets.statusBars)
            )
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .height(56.dp)
                    .padding(horizontal = 12.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                // ── Logo (icon only on mobile, matching web's hidden sm:inline) ──
                Image(
                    painter = painterResource(R.drawable.logo),
                    contentDescription = "Simple Photos",
                    modifier = Modifier
                        .size(32.dp)
                        .clip(RoundedCornerShape(6.dp))
                        .clickable(onClick = navigation.onGalleryClick)
                )

                Spacer(Modifier.width(8.dp))

                // ── Nav Tabs ─────────────────────────────────────────
                NavTab(
                    iconRes = R.drawable.ic_image,
                    label = "Gallery",
                    isActive = activeTab == ActiveTab.GALLERY,
                    activeTabBg = activeTabBg,
                    activeTextColor = activeTabText,
                    inactiveTextColor = inactiveTextColor,
                    onClick = navigation.onGalleryClick
                )
                NavTab(
                    iconRes = R.drawable.ic_folder,
                    label = "Albums",
                    isActive = activeTab == ActiveTab.ALBUMS,
                    activeTabBg = activeTabBg,
                    activeTextColor = activeTabText,
                    inactiveTextColor = inactiveTextColor,
                    onClick = navigation.onAlbumsClick
                )
                NavTab(
                    iconRes = R.drawable.ic_magnify_glass,
                    label = "Search",
                    isActive = activeTab == ActiveTab.SEARCH,
                    activeTabBg = activeTabBg,
                    activeTextColor = activeTabText,
                    inactiveTextColor = inactiveTextColor,
                    onClick = navigation.onSearchClick
                )
                NavTab(
                    iconRes = R.drawable.ic_trashcan,
                    label = "Trash",
                    isActive = activeTab == ActiveTab.TRASH,
                    activeTabBg = activeTabBg,
                    activeTextColor = activeTabText,
                    inactiveTextColor = inactiveTextColor,
                    onClick = navigation.onTrashClick
                )

                // ── Page-specific actions ────────────────────────────
                children()

                // ── Spacer ───────────────────────────────────────────
                Spacer(Modifier.weight(1f))

                // ── Sync indicator (hidden on mobile, matching web's hidden sm:inline) ──
                // On mobile the web hides the text label; we just skip it.

                // ── Separator + Avatar + Dropdown ─────────────────────
                // Vertical divider matching web's border-l border-white/10
                Box(
                    Modifier
                        .padding(horizontal = 4.dp)
                        .height(24.dp)
                        .width(1.dp)
                        .background(borderColor)
                )
                UserMenu(
                    username = username,
                    isSyncing = isSyncing,
                    inactiveTextColor = inactiveTextColor,
                    onSecureGalleryClick = navigation.onSecureGalleryClick,
                    onSharedAlbumsClick = navigation.onSharedAlbumsClick,
                    onSettingsClick = navigation.onSettingsClick,
                    onDiagnosticsClick = navigation.onDiagnosticsClick,
                    isAdmin = navigation.isAdmin,
                    onLogout = navigation.onLogout,
                    onToggleTheme = navigation.onToggleTheme
                )
            }
            // Bottom border
            Box(
                Modifier
                    .fillMaxWidth()
                    .height(1.dp)
                    .background(borderColor)
            )
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// Nav tab button
// ─────────────────────────────────────────────────────────────────────

@Composable
private fun NavTab(
    iconRes: Int,
    label: String,
    isActive: Boolean,
    activeTabBg: Color,
    activeTextColor: Color,
    inactiveTextColor: Color,
    onClick: () -> Unit,
) {
    val bgColor = if (isActive) activeTabBg else Color.Transparent
    val contentColor = if (isActive) activeTextColor else inactiveTextColor

    // Icon-only on mobile (matching web's "hidden md:inline" on label text)
    Surface(
        modifier = Modifier
            .padding(horizontal = 1.dp)
            .clip(RoundedCornerShape(6.dp))
            .clickable(onClick = onClick),
        color = bgColor,
        shape = RoundedCornerShape(6.dp)
    ) {
        Box(
            modifier = Modifier.padding(horizontal = 8.dp, vertical = 6.dp),
            contentAlignment = Alignment.Center
        ) {
            Icon(
                painter = painterResource(iconRes),
                contentDescription = label,
                tint = contentColor,
                modifier = Modifier.size(18.dp)
            )
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// User avatar + dropdown menu
// ─────────────────────────────────────────────────────────────────────

/**
 * RGB colours matching the web's conic-gradient processing ring.
 */
private val rgbRingColors = listOf(
    Color(0xFFFF0000),
    Color(0xFFFF8800),
    Color(0xFFFFFF00),
    Color(0xFF00FF00),
    Color(0xFF0088FF),
    Color(0xFF8800FF),
    Color(0xFFFF0088),
    Color(0xFFFF0000), // wrap back to start for smooth loop
)

@Composable
private fun UserMenu(
    username: String,
    isSyncing: Boolean,
    inactiveTextColor: Color,
    onSecureGalleryClick: () -> Unit,
    onSharedAlbumsClick: () -> Unit = {},
    onSettingsClick: () -> Unit,
    onDiagnosticsClick: () -> Unit = {},
    isAdmin: Boolean = false,
    onLogout: () -> Unit,
    onToggleTheme: () -> Unit = {},
) {
    var expanded by remember { mutableStateOf(false) }

    // Avatar gradient matching web (blue-500 → purple-600)
    val avatarGradient = Brush.linearGradient(
        colors = listOf(Color(0xFF3B82F6), Color(0xFF9333EA)),
        start = Offset(0f, 0f),
        end = Offset(100f, 100f)
    )

    // Infinite rotation for the processing ring (mirrors the web's rgb-spin animation)
    val infiniteTransition = rememberInfiniteTransition(label = "syncRing")
    val rotation by infiniteTransition.animateFloat(
        initialValue = 0f,
        targetValue = 360f,
        animationSpec = infiniteRepeatable(
            animation = tween(durationMillis = 1500, easing = LinearEasing),
            repeatMode = RepeatMode.Restart
        ),
        label = "ringRotation"
    )

    Box {
        // Avatar with optional spinning ring
        Box(
            modifier = Modifier
                .size(34.dp) // slightly larger to accommodate ring padding
                .clickable { expanded = !expanded },
            contentAlignment = Alignment.Center
        ) {
            // Draw the spinning RGB ring when syncing
            if (isSyncing) {
                Canvas(modifier = Modifier.matchParentSize()) {
                    val ringWidth = 3.dp.toPx()
                    val radius = (size.minDimension - ringWidth) / 2f
                    rotate(rotation) {
                        drawCircle(
                            brush = Brush.sweepGradient(rgbRingColors),
                            radius = radius,
                            style = Stroke(width = ringWidth, cap = StrokeCap.Round)
                        )
                    }
                }
            }

            // Avatar circle
            Box(
                modifier = Modifier
                    .size(28.dp)
                    .clip(CircleShape)
                    .background(avatarGradient),
                contentAlignment = Alignment.Center
            ) {
                Text(
                    text = username.take(1).uppercase(),
                    color = Color.White,
                    fontSize = 12.sp,
                    fontWeight = FontWeight.Bold
                )
            }
        }

        // Override MaterialTheme shapes.extraSmall to apply rounded corners
        // to the dropdown menu popup (Material3 DropdownMenu uses extraSmall shape)
        MaterialTheme(
            shapes = MaterialTheme.shapes.copy(extraSmall = RoundedCornerShape(16.dp))
        ) {
            DropdownMenu(
                expanded = expanded,
                onDismissRequest = { expanded = false },
                offset = DpOffset(0.dp, 4.dp)
            ) {
            DropdownMenuItem(
                text = { Text("Secure Albums") },
                onClick = {
                    expanded = false
                    onSecureGalleryClick()
                },
                leadingIcon = {
                    Icon(painter = painterResource(R.drawable.ic_locks), contentDescription = null, modifier = Modifier.size(18.dp))
                }
            )
            DropdownMenuItem(
                text = { Text("Shared Albums") },
                onClick = {
                    expanded = false
                    onSharedAlbumsClick()
                },
                leadingIcon = {
                    Icon(painter = painterResource(R.drawable.ic_shared), contentDescription = null, modifier = Modifier.size(18.dp))
                }
            )
            DropdownMenuItem(
                text = { Text("Settings") },
                onClick = {
                    expanded = false
                    onSettingsClick()
                },
                leadingIcon = {
                    Icon(painter = painterResource(R.drawable.ic_gear), contentDescription = null, modifier = Modifier.size(18.dp))
                }
            )
            if (isAdmin) {
            DropdownMenuItem(
                text = { Text("Diagnostics") },
                onClick = {
                    expanded = false
                    onDiagnosticsClick()
                },
                leadingIcon = {
                    Icon(painter = painterResource(R.drawable.ic_shield), contentDescription = null, modifier = Modifier.size(18.dp))
                }
            )
            }
            HorizontalDivider()
            DropdownMenuItem(
                text = {
                    val systemDark = androidx.compose.foundation.isSystemInDarkTheme()
                    val isDark = ThemeState.isDark(systemDark)
                    Text(if (isDark) "Dark Mode" else "Light Mode")
                },
                onClick = {
                    expanded = false
                    onToggleTheme()
                },
                leadingIcon = {
                    val systemDark = androidx.compose.foundation.isSystemInDarkTheme()
                    val isDark = ThemeState.isDark(systemDark)
                    Icon(
                        painter = painterResource(if (isDark) R.drawable.ic_night else R.drawable.ic_sun),
                        contentDescription = null,
                        modifier = Modifier.size(18.dp)
                    )
                }
            )
            HorizontalDivider()
            DropdownMenuItem(
                text = {
                    Text("Sign Out", color = MaterialTheme.colorScheme.error)
                },
                onClick = {
                    expanded = false
                    onLogout()
                },
                leadingIcon = {
                    Icon(
                        painter = painterResource(R.drawable.ic_right_arrow),
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.error,
                        modifier = Modifier.size(18.dp)
                    )
                }
            )
        }
        } // end MaterialTheme shapes override
    }
}
