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
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.SweepGradient
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.rotate
import androidx.compose.ui.graphics.vector.ImageVector
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
enum class ActiveTab { GALLERY, ALBUMS, TRASH }

/**
 * Navigation callback bundle passed into [AppHeader].
 */
data class HeaderNavigation(
    val onGalleryClick: () -> Unit = {},
    val onAlbumsClick: () -> Unit = {},
    val onTrashClick: () -> Unit = {},
    val onSettingsClick: () -> Unit = {},
    val onLogout: () -> Unit = {},
    val onThemeToggle: () -> Unit = {},
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
    // ── Colors matching web ────────────────────────────────────────
    val headerGradient = Brush.horizontalGradient(
        colors = listOf(
            Color(0xFF111827), // gray-900
            Color(0xFF1F2937), // gray-800
            Color(0xFF111827), // gray-900
        )
    )
    val borderColor = Color.White.copy(alpha = 0.1f)
    val inactiveTextColor = Color(0xFF9CA3AF) // gray-400
    val activeTabBg = Color.White.copy(alpha = 0.15f)

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
                    icon = Icons.Default.Image,
                    label = "Gallery",
                    isActive = activeTab == ActiveTab.GALLERY,
                    activeTabBg = activeTabBg,
                    inactiveTextColor = inactiveTextColor,
                    onClick = navigation.onGalleryClick
                )
                NavTab(
                    icon = Icons.Default.Folder,
                    label = "Albums",
                    isActive = activeTab == ActiveTab.ALBUMS,
                    activeTabBg = activeTabBg,
                    inactiveTextColor = inactiveTextColor,
                    onClick = navigation.onAlbumsClick
                )
                NavTab(
                    icon = Icons.Default.Delete,
                    label = "Trash",
                    isActive = activeTab == ActiveTab.TRASH,
                    activeTabBg = activeTabBg,
                    inactiveTextColor = inactiveTextColor,
                    onClick = navigation.onTrashClick
                )

                // ── Page-specific actions ────────────────────────────
                children()

                // ── Spacer ───────────────────────────────────────────
                Spacer(Modifier.weight(1f))

                // ── Sync indicator (hidden on mobile, matching web's hidden sm:inline) ──
                // On mobile the web hides the text label; we just skip it.

                // ── Theme toggle ─────────────────────────────────────
                val isDark = ThemeState.mode == "dark" ||
                    (ThemeState.mode == "system") // on dark header, show moon
                IconButton(
                    onClick = navigation.onThemeToggle,
                    modifier = Modifier.size(36.dp)
                ) {
                    Icon(
                        imageVector = if (ThemeState.mode == "dark" || (ThemeState.mode == "system"))
                            Icons.Default.LightMode else Icons.Default.DarkMode,
                        contentDescription = "Toggle theme",
                        tint = inactiveTextColor,
                        modifier = Modifier.size(20.dp)
                    )
                }

                // ── Divider ──────────────────────────────────────────
                Box(
                    Modifier
                        .padding(horizontal = 4.dp)
                        .width(1.dp)
                        .height(24.dp)
                        .background(borderColor)
                )

                // ── Avatar + Dropdown ────────────────────────────────
                UserMenu(
                    username = username,
                    isSyncing = isSyncing,
                    inactiveTextColor = inactiveTextColor,
                    onSettingsClick = navigation.onSettingsClick,
                    onLogout = navigation.onLogout
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
    icon: ImageVector,
    label: String,
    isActive: Boolean,
    activeTabBg: Color,
    inactiveTextColor: Color,
    onClick: () -> Unit,
) {
    val bgColor = if (isActive) activeTabBg else Color.Transparent
    val contentColor = if (isActive) Color.White else inactiveTextColor

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
                icon,
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
    onSettingsClick: () -> Unit,
    onLogout: () -> Unit,
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

        DropdownMenu(
            expanded = expanded,
            onDismissRequest = { expanded = false },
            offset = DpOffset(0.dp, 4.dp)
        ) {
            DropdownMenuItem(
                text = { Text("Settings") },
                onClick = {
                    expanded = false
                    onSettingsClick()
                },
                leadingIcon = {
                    Icon(Icons.Default.Settings, contentDescription = null, modifier = Modifier.size(18.dp))
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
                        Icons.Default.Logout,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.error,
                        modifier = Modifier.size(18.dp)
                    )
                }
            )
        }
    }
}
