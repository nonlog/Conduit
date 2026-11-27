package com.conduit.sync

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialExpressiveTheme
import androidx.compose.material3.MotionScheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp
import androidx.core.view.WindowCompat

/** Public Material 3 Expressive API with authentic tonal surfaces and native dynamic color. */
@Composable
internal fun ConduitTheme(
    darkOverride: Boolean? = null,
    motionOverride: Boolean? = null,
    content: @Composable () -> Unit,
) {
    val dark =
        darkOverride ?: when (Settings.themeMode) {
            "dark" -> true
            "light" -> false
            else -> isSystemInDarkTheme()
        }
    val context = LocalContext.current
    val view = LocalView.current
    SideEffect {
        val activity =
            generateSequence<Context>(view.context) { current ->
                (current as? ContextWrapper)?.baseContext?.takeIf { it !== current }
            }.filterIsInstance<Activity>().firstOrNull()
        if (activity != null) {
            val bars = WindowCompat.getInsetsController(activity.window, view)
            bars.isAppearanceLightStatusBars = !dark
            bars.isAppearanceLightNavigationBars = !dark
        }
    }
    val colors =
        when {
            Settings.wallpaperAccent && Build.VERSION.SDK_INT >= 31 -> {
                if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
            }

            dark -> {
                darkColorScheme(
                    primary = Color(0xFFA8C7FA),
                    onPrimary = Color(0xFF062E6F),
                    primaryContainer = Color(0xFF0842A0),
                    onPrimaryContainer = Color(0xFFD3E3FD),
                    inversePrimary = Color(0xFF0B57D0),
                    secondary = Color(0xFFBEC6DC),
                    onSecondary = Color(0xFF283041),
                    secondaryContainer = Color(0xFF2B3447),
                    onSecondaryContainer = Color(0xFFDAE2F9),
                    tertiary = Color(0xFFD8BAFF),
                    onTertiary = Color(0xFF391F5A),
                    tertiaryContainer = Color(0xFF4F3473),
                    onTertiaryContainer = Color(0xFFEEDBFF),
                    error = Color(0xFFFFB4AB),
                    onError = Color(0xFF690005),
                    errorContainer = Color(0xFF93000A),
                    onErrorContainer = Color(0xFFFFDAD6),
                    background = Color(0xFF101218),
                    onBackground = Color(0xFFE3E6F0),
                    surface = Color(0xFF101218),
                    onSurface = Color(0xFFE3E6F0),
                    surfaceVariant = Color(0xFF252A36),
                    onSurfaceVariant = Color(0xFFBDC3D4),
                    outline = Color(0xFF8C92A4),
                    outlineVariant = Color(0xFF3D4354),
                    scrim = Color.Black,
                    inverseSurface = Color(0xFFE3E6F0),
                    inverseOnSurface = Color(0xFF2B2F3A),
                    surfaceDim = Color(0xFF0D0F14),
                    surfaceBright = Color(0xFF353A47),
                    surfaceContainerLowest = Color(0xFF0B0D12),
                    surfaceContainerLow = Color(0xFF171A23),
                    surfaceContainer = Color(0xFF1D212C),
                    surfaceContainerHigh = Color(0xFF262B38),
                    surfaceContainerHighest = Color(0xFF303646),
                )
            }

            else -> {
                lightColorScheme(
                    primary = Color(0xFF0B57D0),
                    onPrimary = Color.White,
                    primaryContainer = Color(0xFFD3E3FD),
                    onPrimaryContainer = Color(0xFF041E49),
                    inversePrimary = Color(0xFFA8C7FA),
                    secondary = Color(0xFF565E71),
                    onSecondary = Color.White,
                    secondaryContainer = Color(0xFFDAE2F9),
                    onSecondaryContainer = Color(0xFF131C2B),
                    tertiary = Color(0xFF684D8C),
                    onTertiary = Color.White,
                    tertiaryContainer = Color(0xFFEEDBFF),
                    onTertiaryContainer = Color(0xFF230844),
                    error = Color(0xFFBA1A1A),
                    onError = Color.White,
                    errorContainer = Color(0xFFFFDAD6),
                    onErrorContainer = Color(0xFF410002),
                    background = Color(0xFFF8F9FD),
                    onBackground = Color(0xFF191C22),
                    surface = Color(0xFFF8F9FD),
                    onSurface = Color(0xFF191C22),
                    surfaceVariant = Color(0xFFE1E5F0),
                    onSurfaceVariant = Color(0xFF44474F),
                    outline = Color(0xFF747780),
                    outlineVariant = Color(0xFFC4C6D0),
                    scrim = Color.Black,
                    inverseSurface = Color(0xFF2E3138),
                    inverseOnSurface = Color(0xFFF0F1F7),
                    surfaceDim = Color(0xFFD9DCE5),
                    surfaceBright = Color(0xFFF8F9FD),
                    surfaceContainerLowest = Color.White,
                    surfaceContainerLow = Color(0xFFF2F4FA),
                    surfaceContainer = Color(0xFFECEFF7),
                    surfaceContainerHigh = Color(0xFFE6EAF3),
                    surfaceContainerHighest = Color(0xFFE0E4EE),
                )
            }
        }
    val motionEnabled = motionOverride ?: rememberSystemMotionEnabled()
    CompositionLocalProvider(LocalConduitMotion provides motionEnabled) {
        MaterialExpressiveTheme(
            colorScheme = colors,
            motionScheme = MotionScheme.expressive(),
            typography =
                Typography(
                    headlineLarge = TextStyle(fontSize = 24.sp, lineHeight = 32.sp, fontWeight = FontWeight.SemiBold),
                    headlineMedium = TextStyle(fontSize = 22.sp, lineHeight = 28.sp, fontWeight = FontWeight.SemiBold),
                    titleLarge = TextStyle(fontSize = 20.sp, lineHeight = 26.sp, fontWeight = FontWeight.Medium),
                    titleMedium = TextStyle(fontSize = 16.sp, lineHeight = 22.sp, fontWeight = FontWeight.SemiBold),
                    titleSmall = TextStyle(fontSize = 14.sp, lineHeight = 20.sp, fontWeight = FontWeight.Medium),
                    bodyLarge = TextStyle(fontSize = 17.sp, lineHeight = 24.sp),
                    bodyMedium = TextStyle(fontSize = 14.sp, lineHeight = 20.sp),
                    bodySmall = TextStyle(fontSize = 12.sp, lineHeight = 17.sp),
                    labelLarge = TextStyle(fontSize = 14.sp, lineHeight = 20.sp, fontWeight = FontWeight.Medium),
                    labelMedium = TextStyle(fontSize = 12.sp, lineHeight = 16.sp, fontWeight = FontWeight.Medium),
                    labelSmall = TextStyle(fontSize = 11.sp, lineHeight = 15.sp),
                ),
            content = content,
        )
    }
}
