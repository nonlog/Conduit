@file:OptIn(androidx.compose.material3.ExperimentalMaterial3ExpressiveApi::class)

package com.conduit.sync

import androidx.compose.animation.core.animateDpAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

internal object ExpressiveLayout {
    val pageInset = 16.dp
    val cardCorner = 28.dp
    val cardSpacing = 10.dp
    val dockHeight = 62.dp
}

/** Quiet neutral surfaces; no outlines or fake glass gradients. */
@Composable
internal fun ExpressiveCard(modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(ExpressiveLayout.cardCorner),
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        shadowElevation = 1.dp,
        content = content,
    )
}

@Composable
internal fun ExpressiveSectionTitle(title: String, detail: String? = null) {
    Row(
        Modifier.fillMaxWidth().padding(start = 16.dp, end = 12.dp, top = 14.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            title, modifier = Modifier.weight(1f),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1, overflow = TextOverflow.Ellipsis,
        )
        if (!detail.isNullOrBlank()) {
            Text(detail, style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

/** Single-weight page title and a compact tonal action, as in the supplied reference. */
@Composable
internal fun ExpressiveTopBar(title: String, action: (() -> Unit)? = null, back: (() -> Unit)? = null) {
    Row(
        Modifier.fillMaxWidth().statusBarsPadding().heightIn(min = 84.dp)
            .padding(horizontal = 16.dp, vertical = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (back != null) {
            IconButton(onClick = back) {
                Icon(painterResource(R.drawable.ic_arrow_back), "Back", modifier = Modifier.size(22.dp))
            }
        }
        Text(title, Modifier.weight(1f), style = MaterialTheme.typography.headlineLarge,
            maxLines = 1, overflow = TextOverflow.Ellipsis)
        if (action != null) {
            FilledTonalIconButton(
                onClick = action,
                modifier = Modifier.size(44.dp),
                colors = IconButtonDefaults.filledTonalIconButtonColors(
                    containerColor = MaterialTheme.colorScheme.surfaceContainerLow,
                    contentColor = MaterialTheme.colorScheme.onSurface,
                ),
            ) {
                Icon(painterResource(R.drawable.ic_add), "Pair desktop", Modifier.size(24.dp))
            }
        }
    }
}

/** Compact floating bar. Selection uses the official Expressive spatial motion token. */
@Composable
internal fun ExpressiveNavigationDock(
    selectedSettings: Boolean, onHome: () -> Unit, onSettings: () -> Unit,
    destination: String = if (selectedSettings) "Settings" else "Home",
    onTransfers: () -> Unit = {}, onClipboard: () -> Unit = {},
) {
    val selectedIndex = listOf("Home", "Transfers", "Clipboard", "Settings").indexOf(destination).coerceAtLeast(0)
    Box(
        Modifier.fillMaxWidth().navigationBarsPadding().padding(horizontal = 20.dp, vertical = 6.dp),
        contentAlignment = Alignment.Center,
    ) {
        Surface(
            modifier = Modifier.widthIn(max = 320.dp).fillMaxWidth(),
            shape = CircleShape,
            color = MaterialTheme.colorScheme.surfaceContainer,
            shadowElevation = 6.dp,
        ) {
            BoxWithConstraints(Modifier.height(ExpressiveLayout.dockHeight).padding(4.dp)) {
                val cell = maxWidth / 4
                val offset by animateDpAsState(
                    targetValue = cell * selectedIndex,
                    animationSpec = MaterialTheme.motionScheme.defaultSpatialSpec(),
                    label = "Navigation selection",
                )
                Box(Modifier.offset(x = offset).width(cell).fillMaxHeight()
                    .clip(CircleShape).background(MaterialTheme.colorScheme.surfaceContainerHighest))
                Row(Modifier.fillMaxSize().selectableGroup()) {
                    DockItem("Home", R.drawable.ic_dashboard, selectedIndex == 0, onHome, Modifier.weight(1f))
                    DockItem("Transfers", R.drawable.ic_transfer, selectedIndex == 1, onTransfers, Modifier.weight(1f))
                    DockItem("Clipboard", R.drawable.ic_file_outline, selectedIndex == 2, onClipboard, Modifier.weight(1f))
                    DockItem("Settings", R.drawable.ic_settings_outline, selectedIndex == 3, onSettings, Modifier.weight(1f))
                }
            }
        }
    }
}

@Composable
private fun DockItem(label: String, icon: Int, selected: Boolean, onClick: () -> Unit, modifier: Modifier) {
    val color = if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant
    Column(
        modifier.fillMaxHeight().clip(CircleShape)
            .selectable(selected = selected, onClick = onClick, role = Role.Tab)
            .semantics(mergeDescendants = true) { contentDescription = label },
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Icon(painterResource(icon), null, Modifier.size(24.dp), tint = color)
        Spacer(Modifier.height(2.dp))
        Text(label, style = MaterialTheme.typography.labelMedium, color = color,
            fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}
