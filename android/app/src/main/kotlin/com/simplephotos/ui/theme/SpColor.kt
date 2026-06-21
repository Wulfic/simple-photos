/**
 * Shared design tokens — the Compose mirror of the web design system
 * (web/tailwind.config.js + web/src/index.css @layer base).
 *
 * Goal: visual CONSISTENCY between the web app and the Android app. The web
 * re-themed onto a single **violet** accent plus a semantic surface/text/edge
 * ramp (light = cool slate, dark = gray); this file ports the same palette so
 * Compose screens stop drifting from the website.
 *
 *  - [Violet]    : the accent ramp (was ad-hoc blues on Android).
 *  - [SpColors]  : semantic light/dark tokens (canvas/surface/edge/fg…) exposed
 *                  through [LocalSpColors]. Use these on normal light/dark
 *                  screens instead of hardcoded grays.
 *  - [SpViewer]  : the ALWAYS-dark palette for media-viewer overlays (info /
 *                  tag / edit panels). The viewer is dark in every theme, so it
 *                  mirrors the web viewer's hardcoded gray-900 / white / violet
 *                  rather than the swap-on-`.dark` tokens.
 */
package com.simplephotos.ui.theme

import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

/** Tailwind `violet` ramp — the single accent source (web: `accent: colors.violet`). */
object Violet {
    val v50 = Color(0xFFF5F3FF)
    val v100 = Color(0xFFEDE9FE)
    val v200 = Color(0xFFDDD6FE)
    val v300 = Color(0xFFC4B5FD)
    val v400 = Color(0xFFA78BFA)
    val v500 = Color(0xFF8B5CF6)
    val v600 = Color(0xFF7C3AED) // primary action
    val v700 = Color(0xFF6D28D9)
    val v800 = Color(0xFF5B21B6)
    val v900 = Color(0xFF4C1D95)
}

/**
 * Semantic surface / text / edge tokens. Two instances ([SpLightColors] /
 * [SpDarkColors]) mirror the web `:root` and `.dark` ramps 1:1, so a call site
 * reads `LocalSpColors.current.surface` once instead of branching on theme.
 */
data class SpColors(
    val canvas: Color,
    val surface: Color,
    val surfaceRaised: Color,
    val surfaceSunken: Color,
    val edge: Color,
    val edgeStrong: Color,
    val fg: Color,
    val fgMuted: Color,
    val fgSubtle: Color,
    // The accent is theme-independent (same violet in light + dark) but lives
    // here so call sites have one place to read every brand color.
    val accent: Color = Violet.v600,
    val accentHover: Color = Violet.v500,
    val onAccent: Color = Color.White,
    val isDark: Boolean,
)

/** Web `:root` — cool-neutral slate ramp (off-white canvas, AA-passing text). */
val SpLightColors = SpColors(
    canvas = Color(0xFFF8FAFC),        // slate-50
    surface = Color(0xFFFFFFFF),       // white
    surfaceRaised = Color(0xFFF8FAFC), // slate-50
    surfaceSunken = Color(0xFFF1F5F9), // slate-100
    edge = Color(0xFFE2E8F0),          // slate-200
    edgeStrong = Color(0xFFCBD5E1),    // slate-300
    fg = Color(0xFF0F172A),            // slate-900
    fgMuted = Color(0xFF475569),       // slate-600
    fgSubtle = Color(0xFF64748B),      // slate-500
    accent = Violet.v600,
    accentHover = Violet.v700,
    isDark = false,
)

/** Web `.dark` — the original hand-tuned gray ramp, preserved 1:1. */
val SpDarkColors = SpColors(
    canvas = Color(0xFF111827),        // gray-900
    surface = Color(0xFF1F2937),       // gray-800
    surfaceRaised = Color(0xFF374151), // gray-700 (lighter = raised in dark)
    surfaceSunken = Color(0xFF111827), // gray-900
    edge = Color(0xFF374151),          // gray-700
    edgeStrong = Color(0xFF4B5563),    // gray-600
    fg = Color(0xFFF3F4F6),            // gray-100
    fgMuted = Color(0xFFD1D5DB),       // gray-300
    fgSubtle = Color(0xFF9CA3AF),      // gray-400
    accent = Violet.v500,              // a touch lighter reads better on dark
    accentHover = Violet.v400,
    isDark = true,
)

/** Provided by `SimplePhotosTheme`; defaults to dark so viewer overlays are safe
 *  even if read outside the theme. */
val LocalSpColors = staticCompositionLocalOf { SpDarkColors }

/**
 * ALWAYS-dark palette for media-viewer overlays (info / tag / edit panels).
 * Mirrors the hardcoded values in web/src/components/viewer/PhotoInfoPanel.tsx
 * (`bg-gray-900/95`, `bg-gray-800` inputs, `border-white/10`, `text-gray-400`,
 * `text-accent-400` links, `bg-red-900/50` errors) so the panels match the web
 * viewer exactly regardless of the app's light/dark setting.
 */
object SpViewer {
    val panelBg = Color(0xF2111827)            // gray-900 @ 95%
    val inputBg = Color(0xFF1F2937)            // gray-800
    val inputBorder = Color(0x1AFFFFFF)        // white/10
    val divider = Color(0x1AFFFFFF)            // white/10
    val textPrimary = Color.White
    val textMuted = Color(0xFF9CA3AF)          // gray-400
    val textSubtle = Color(0xFF6B7280)         // gray-500
    val textFaint = Color(0xFFD1D5DB)          // gray-300 (raw-EXIF values)
    val accent = Violet.v400                   // accent-400 links
    val accentHover = Violet.v300              // accent-300
    val dangerBg = Color(0x80450A0A)           // red-900 @ 50%
    val dangerText = Color(0xFFFCA5A5)         // red-300
    val secondaryBtn = Color(0xFF374151)       // gray-700
    val secondaryBtnHover = Color(0xFF4B5563)  // gray-600
}
