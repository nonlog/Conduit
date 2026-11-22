package com.conduit.sync

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.ColorScheme
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

private fun ColorScheme.withDarkSurfaces(): ColorScheme =
    copy(
        background = Color(0xFF101413),
        onBackground = Color(0xFFE0E4E3),
        surface = Color(0xFF101413),
        onSurface = Color(0xFFE0E4E3),
        surfaceVariant = Color(0xFF434A49),
        onSurfaceVariant = Color(0xFFC6D0CF),
        outline = Color(0xFFACB7B6),
        outlineVariant = Color(0xFF5C6665),
        surfaceDim = Color(0xFF101413),
        surfaceBright = Color(0xFF505857),
        surfaceContainerLowest = Color(0xFF0B0F0F),
        surfaceContainerLow = Color(0xFF343A39),
        surfaceContainer = Color(0xFF434A49),
        surfaceContainerHigh = Color(0xFF4A5150),
        surfaceContainerHighest = Color(0xFF505857),
    )

/** Public Material 3 Expressive API, not a local imitation of MaterialTheme. */
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
    val palette =
        when {
            Settings.wallpaperAccent && Build.VERSION.SDK_INT >= 31 -> {
                if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
            }

            dark -> {
                darkColorScheme(
                    primary = Color(0xFF4DDADB),
                    onPrimary = Color(0xFF003738),
                    primaryContainer = Color(0xFF005052),
                    onPrimaryContainer = Color(0xFF9CF0EF),
                    inversePrimary = Color(0xFF006A6C),
                    secondary = Color(0xFFB1CCCB),
                    onSecondary = Color(0xFF1C3535),
                    secondaryContainer = Color(0xFF334B4B),
                    onSecondaryContainer = Color(0xFFCDE8E7),
                    tertiary = Color(0xFFB6C8E8),
                    onTertiary = Color(0xFF1E314C),
                    tertiaryContainer = Color(0xFF354762),
                    onTertiaryContainer = Color(0xFFD5E3FF),
                    error = Color(0xFFFFB4AB),
                    onError = Color(0xFF690005),
                    errorContainer = Color(0xFF93000A),
                    onErrorContainer = Color(0xFFFFDAD6),
                    scrim = Color.Black,
                    inverseSurface = Color(0xFFE0E4E3),
                    inverseOnSurface = Color(0xFF2C3131),
                )
            }

            else -> {
                lightColorScheme(
                    primary = Color(0xFF006A6C),
                    onPrimary = Color.White,
                    primaryContainer = Color(0xFF9CF0EF),
                    onPrimaryContainer = Color(0xFF002021),
                    inversePrimary = Color(0xFF4DDADB),
                    secondary = Color(0xFF4A6363),
                    onSecondary = Color.White,
                    secondaryContainer = Color(0xFFCDE8E7),
                    onSecondaryContainer = Color(0xFF051F1F),
                    tertiary = Color(0xFF4D5F7A),
                    onTertiary = Color.White,
                    tertiaryContainer = Color(0xFFD5E3FF),
                    onTertiaryContainer = Color(0xFF081C35),
                    error = Color(0xFFBA1A1A),
                    onError = Color.White,
                    errorContainer = Color(0xFFFFDAD6),
                    onErrorContainer = Color(0xFF410002),
                    background = Color(0xFFF7FAF9),
                    onBackground = Color(0xFF171D1D),
                    surface = Color(0xFFF7FAF9),
                    onSurface = Color(0xFF171D1D),
                    surfaceVariant = Color(0xFFDAE5E3),
                    onSurfaceVariant = Color(0xFF3F4948),
                    outline = Color(0xFF6F7978),
                    outlineVariant = Color(0xFFBFC9C8),
                    scrim = Color.Black,
                    inverseSurface = Color(0xFF2C3131),
                    inverseOnSurface = Color(0xFFEFF1F0),
                    surfaceDim = Color(0xFFD8DAD9),
                    surfaceBright = Color(0xFFF7FAF9),
                    surfaceContainerLowest = Color.White,
                    surfaceContainerLow = Color(0xFFF1F4F3),
                    surfaceContainer = Color(0xFFEBEFEE),
                    surfaceContainerHigh = Color(0xFFE5E9E8),
                    surfaceContainerHighest = Color(0xFFDFE3E2),
                )
            }
        }
    // Wallpaper colors can change the accents, but should not flatten dark card elevation.
    val colors = if (dark) palette.withDarkSurfaces() else palette
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
