package com.simplephotos.ui.components

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
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
        Column {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .background(headerGradient)
                    .height(56.dp)
                    .padding(horizontal = 12.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                // ── Logo + Brand ─────────────────────────────────────
                Row(
                    modifier = Modifier
                        .clickable(onClick = navigation.onGalleryClick)
                        .padding(end = 8.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Image(
                        painter = painterResource(R.drawable.logo),
                        contentDescription = "Simple Photos",
                        modifier = Modifier
                            .size(32.dp)
                            .clip(RoundedCornerShape(6.dp))
                    )
                    Spacer(Modifier.width(8.dp))
                    Text(
                        "Simple Photos",
                        color = Color.White,
                        fontWeight = FontWeight.SemiBold,
                        fontSize = 16.sp,
                        maxLines = 1
                    )
                }

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
                    icon = Icons.Default.PhotoAlbum,
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

                // ── Sync indicator ───────────────────────────────────
                if (isSyncing && syncLabel != null) {
                    Text(
                        "$syncLabel…",
                        color = Color(0xFF93C5FD), // blue-300
                        fontSize = 11.sp,
                        fontWeight = FontWeight.Medium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.padding(end = 8.dp)
                    )
                }

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
                        .padding(horizontal = 8.dp)
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

    Surface(
        modifier = Modifier
            .padding(horizontal = 2.dp)
            .clip(RoundedCornerShape(6.dp))
            .clickable(onClick = onClick),
        color = bgColor,
        shape = RoundedCornerShape(6.dp)
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Icon(
                icon,
                contentDescription = label,
                tint = contentColor,
                modifier = Modifier.size(16.dp)
            )
            Spacer(Modifier.width(4.dp))
            Text(
                label,
                color = contentColor,
                fontSize = 13.sp,
                fontWeight = FontWeight.Medium,
                maxLines = 1
            )
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// User avatar + dropdown menu
// ─────────────────────────────────────────────────────────────────────

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

    Box {
        Row(
            modifier = Modifier
                .clip(RoundedCornerShape(6.dp))
                .clickable { expanded = !expanded }
                .padding(4.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
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
            Spacer(Modifier.width(6.dp))
            Text(
                username,
                color = inactiveTextColor,
                fontSize = 12.sp,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis
            )
            Spacer(Modifier.width(4.dp))
            Icon(
                Icons.Default.KeyboardArrowDown,
                contentDescription = null,
                tint = inactiveTextColor,
                modifier = Modifier.size(14.dp)
            )
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
