/**
 * Shared button recipe — the Compose mirror of the web `.btn` depth system
 * (web/tailwind.config.js boxShadow `btn`/`btn-hover`/`btn-inset` + the
 * depth-design pass): a top→bottom gradient with a lit top edge, a grounding
 * drop shadow, and a press that sinks the button (translate-y) instead of
 * scaling it. Elevation shadows are kept neutral/black so the swappable violet
 * accent never bleeds into them.
 *
 * Variants mirror the web btn variants: primary (violet) / secondary (neutral)
 * / ghost / danger / success. Colors come from [LocalSpColors], so a button
 * inside a subtree that provides `SpDarkColors` (e.g. the always-dark media
 * viewer) renders dark regardless of the app's light/dark setting.
 *
 * Supports an optional [leadingIcon] and a [loading] spinner so it can stand in
 * for the Material `Button`/`OutlinedButton` call sites it replaces.
 */
package com.simplephotos.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.painter.Painter
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.simplephotos.ui.theme.LocalSpColors
import com.simplephotos.ui.theme.Violet

enum class SpButtonVariant { Primary, Secondary, Ghost, Danger, Success }

private data class SpButtonStyle(
    val brush: Brush,
    val content: Color,
    val border: Color,
    val elevated: Boolean,
)

@Composable
fun SpButton(
    text: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    variant: SpButtonVariant = SpButtonVariant.Primary,
    enabled: Boolean = true,
    loading: Boolean = false,
    leadingIcon: Painter? = null,
    fontSize: Int = 13,
) {
    val sp = LocalSpColors.current
    val interaction = remember { MutableInteractionSource() }
    val pressed by interaction.collectIsPressedAsState()
    val shape = RoundedCornerShape(10.dp)

    val style = when (variant) {
        SpButtonVariant.Primary -> SpButtonStyle(
            brush = Brush.verticalGradient(listOf(Violet.v500, Violet.v600)),
            content = Color.White, border = Color.White.copy(alpha = 0.18f), elevated = true,
        )
        SpButtonVariant.Secondary -> SpButtonStyle(
            brush = Brush.verticalGradient(listOf(sp.surfaceRaised, sp.surface)),
            content = sp.fg, border = sp.edge, elevated = true,
        )
        SpButtonVariant.Danger -> SpButtonStyle(
            brush = Brush.verticalGradient(listOf(Color(0xFFEF4444), Color(0xFFDC2626))),
            content = Color.White, border = Color.White.copy(alpha = 0.18f), elevated = true,
        )
        SpButtonVariant.Success -> SpButtonStyle(
            brush = Brush.verticalGradient(listOf(Color(0xFF22C55E), Color(0xFF16A34A))),
            content = Color.White, border = Color.White.copy(alpha = 0.18f), elevated = true,
        )
        SpButtonVariant.Ghost -> SpButtonStyle(
            brush = Brush.verticalGradient(listOf(Color.Transparent, Color.Transparent)),
            content = sp.accent, border = Color.Transparent, elevated = false,
        )
    }

    val clickable = enabled && !loading
    val alpha = if (enabled) 1f else 0.5f
    Row(
        modifier = modifier
            .graphicsLayer {
                // Press sinks the button 1dp (translate-y), no scale.
                translationY = if (pressed && clickable) 1.dp.toPx() else 0f
                this.alpha = alpha
            }
            .then(
                if (style.elevated)
                    Modifier.shadow(
                        elevation = if (pressed) 1.dp else 4.dp,
                        shape = shape,
                        clip = false,
                    )
                else Modifier
            )
            .clip(shape)
            .background(style.brush, shape)
            .border(1.dp, style.border, shape)
            .clickable(
                interactionSource = interaction,
                indication = null,
                enabled = clickable,
                onClick = onClick,
            )
            .padding(horizontal = 16.dp, vertical = 9.dp),
        horizontalArrangement = Arrangement.Center,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (loading) {
            CircularProgressIndicator(
                modifier = Modifier.size((fontSize + 3).dp),
                strokeWidth = 2.dp,
                color = style.content,
            )
            Spacer(Modifier.width(8.dp))
        } else if (leadingIcon != null) {
            Icon(
                painter = leadingIcon,
                contentDescription = null,
                tint = style.content,
                modifier = Modifier.size((fontSize + 5).dp),
            )
            Spacer(Modifier.width(8.dp))
        }
        Text(
            text = text,
            color = style.content,
            fontSize = fontSize.sp,
            fontWeight = FontWeight.SemiBold,
        )
    }
}
