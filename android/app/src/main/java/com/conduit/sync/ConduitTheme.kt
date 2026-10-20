package com.conduit.sync

import android.os.Build
import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import androidx.core.view.WindowCompat
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.platform.LocalView
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialExpressiveTheme
import androidx.compose.material3.MotionScheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

/** Public Material 3 Expressive API, not a local imitation of MaterialTheme. */
@Composable
internal fun ConduitTheme(darkOverride: Boolean? = null, content: @Composable () -> Unit) {
    val dark = darkOverride ?: when (Settings.themeMode) {
        "dark" -> true
        "light" -> false
        else -> isSystemInDarkTheme()
    }
    val context = LocalContext.current
    val view = LocalView.current
    SideEffect {
        val activity = generateSequence<Context>(view.context) { current ->
            (current as? ContextWrapper)?.baseContext?.takeIf { it !== current }
        }.filterIsInstance<Activity>().firstOrNull()
        if (activity != null) {
            val bars = WindowCompat.getInsetsController(activity.window, view)
            bars.isAppearanceLightStatusBars = !dark
            bars.isAppearanceLightNavigationBars = !dark
        }
    }
    val accent = if (Settings.wallpaperAccent && Build.VERSION.SDK_INT >= 31) {
        if (dark) dynamicDarkColorScheme(context) else dynamicLightColorScheme(context)
    } else if (dark) {
        darkColorScheme(primary = Color(0xFF7EC5C6), onPrimary = Color(0xFF002F30))
    } else {
        lightColorScheme(primary = Color(0xFF006A6C), onPrimary = Color.White)
    }
    // The supplied reference uses neutral surfaces. Tint the accent, not every grey,
    // so wallpaper colour does not turn the entire screen into the old green panel.
    val colors = if (dark) accent.copy(
        background = Color(0xFF111414), surface = Color(0xFF111414),
        surfaceDim = Color(0xFF111414), surfaceBright = Color(0xFF333535),
        surfaceContainerLowest = Color(0xFF0D1010), surfaceContainerLow = Color(0xFF1E2020),
        surfaceContainer = Color(0xFF282A2A), surfaceContainerHigh = Color(0xFF2D3030),
        surfaceContainerHighest = Color(0xFF333535), surfaceVariant = Color(0xFF282A2A),
        onSurface = Color(0xFFE1E4E3), onBackground = Color(0xFFE1E4E3),
        onSurfaceVariant = Color(0xFFBCC6C5), outline = Color(0xFF899392),
        outlineVariant = Color(0xFF3C4140), surfaceTint = Color.Transparent,
        secondary = accent.primary, onSecondary = accent.onPrimary,
        primaryContainer = Color(0xFF2B3939), onPrimaryContainer = accent.primary,
        secondaryContainer = Color(0xFF333535), onSecondaryContainer = Color(0xFFE1E4E3),
        tertiaryContainer = Color(0xFF282A2A), onTertiaryContainer = Color(0xFFBCC6C5),
    ) else accent.copy(
        background = Color(0xFFF5F7F6), surface = Color(0xFFF5F7F6),
        surfaceDim = Color(0xFFE0E4E2), surfaceBright = Color.White,
        surfaceContainerLowest = Color.White, surfaceContainerLow = Color.White,
        surfaceContainer = Color(0xFFEBEFED), surfaceContainerHigh = Color(0xFFE5EAE7),
        surfaceContainerHighest = Color(0xFFDFE6E2), surfaceVariant = Color(0xFFEBEFED),
        onSurface = Color(0xFF191D1C), onBackground = Color(0xFF191D1C),
        onSurfaceVariant = Color(0xFF535F5C), outlineVariant = Color(0xFFD5DDDA),
        surfaceTint = Color.Transparent, secondary = accent.primary,
        primaryContainer = Color(0xFFD7ECE9), onPrimaryContainer = Color(0xFF005255),
        secondaryContainer = Color(0xFFDFE6E2), onSecondaryContainer = Color(0xFF26312E),
        tertiaryContainer = Color(0xFFEBEFED), onTertiaryContainer = Color(0xFF535F5C),
    )
    MaterialExpressiveTheme(
        colorScheme = colors,
        motionScheme = MotionScheme.expressive(),
        typography = Typography(
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
