package testapp

import androidx.compose.animation.AnimatedContent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.path
import androidx.compose.ui.unit.dp

/// Task 28 chevron-crash bisect harness. Three layers, identical state
/// model, escalating UI complexity. Run a layer at a time by uncommenting.
///   A: TextButton with `<` / `>` text — no vector icon, no animation.
///   B: A + AnimatedContent around the displayed month.
///   C: B + Material3 IconButton + vector icon (Icons.AutoMirrored).
@Composable
internal fun ChevronBisectCard() {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant
        ),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                "Chevron bisect (task 28 SIGILL hunt)",
                style = MaterialTheme.typography.titleSmall,
            )
            // Pick a layer to investigate next. A-D survive; E (TooltipBox)
            // SIGILLs ~5 s after first tap — see feedback_tooltip_sigill_wasi.md.
            LayerAPlainText()
            // LayerBAnimatedContent()
            // LayerCIconButtonText()
            // LayerDVectorIconButton()
            // LayerETooltipChevrons()  // ⚠ crashes
        }
    }
}

/// Layer A — plain text buttons, no animation, no vector. If this crashes
/// when tapped repeatedly → bug is in the simplest state-update +
/// recomposition path. If it survives → bug is something on top.
@Composable
private fun LayerAPlainText() {
    var month by remember { mutableIntStateOf(0) }
    Text("Layer A — plain text buttons", style = MaterialTheme.typography.labelMedium)
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        TextButton(onClick = { month-- }) { Text("<") }
        Text("month = $month", modifier = Modifier.padding(horizontal = 8.dp))
        TextButton(onClick = { month++ }) { Text(">") }
    }
}

/// Layer B — adds AnimatedContent. The same chevron-tap interaction now
/// runs a content transition each tap.
@Composable
private fun LayerBAnimatedContent() {
    var month by remember { mutableIntStateOf(0) }
    Text("Layer B — + AnimatedContent", style = MaterialTheme.typography.labelMedium)
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        TextButton(onClick = { month-- }) { Text("<") }
        AnimatedContent(targetState = month, label = "month") { m ->
            Text("month = $m", modifier = Modifier.padding(horizontal = 8.dp))
        }
        TextButton(onClick = { month++ }) { Text(">") }
    }
}

/// Layer E — exact `IconButtonWithTooltip` shape from Material3
/// DatePicker.kt (TooltipBox + PlainTooltip + rememberTooltipState +
/// IconButton + vector Icon). If THIS crashes, the trigger is the
/// TooltipBox/Popup machinery, not the icon or the animation.
@OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
@Composable
private fun LayerETooltipChevrons() {
    var month by remember { mutableIntStateOf(0) }
    Text("Layer E — + TooltipBox (matches DatePicker)",
        style = MaterialTheme.typography.labelMedium)
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        androidx.compose.material3.TooltipBox(
            positionProvider = androidx.compose.material3.TooltipDefaults
                .rememberTooltipPositionProvider(
                    androidx.compose.material3.TooltipAnchorPosition.Above
                ),
            tooltip = { Text("prev") },
            state = androidx.compose.material3.rememberTooltipState(),
        ) {
            IconButton(onClick = { month-- }) {
                Icon(imageVector = chevronLeft, contentDescription = "prev")
            }
        }
        AnimatedContent(targetState = month, label = "month") { m ->
            Text("month = $m", modifier = Modifier.padding(horizontal = 8.dp))
        }
        androidx.compose.material3.TooltipBox(
            positionProvider = androidx.compose.material3.TooltipDefaults
                .rememberTooltipPositionProvider(
                    androidx.compose.material3.TooltipAnchorPosition.Above
                ),
            tooltip = { Text("next") },
            state = androidx.compose.material3.rememberTooltipState(),
        ) {
            IconButton(onClick = { month++ }) {
                Icon(imageVector = chevronRight, contentDescription = "next")
            }
        }
    }
}

/// Layer C — Material3 IconButton with a plain `Text("<")` child.
/// Exercises the IconButton + ripple machinery without going through
/// the vector-icon rasterization path. If this crashes → bug is in
/// IconButton/ripple. If it survives → bug is specifically in vector
/// rasterization.
@Composable
private fun LayerCIconButtonText() {
    var month by remember { mutableIntStateOf(0) }
    Text("Layer C — IconButton + Text label", style = MaterialTheme.typography.labelMedium)
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        IconButton(onClick = { month-- }) { Text("<") }
        AnimatedContent(targetState = month, label = "month") { m ->
            Text("month = $m", modifier = Modifier.padding(horizontal = 8.dp))
        }
        IconButton(onClick = { month++ }) { Text(">") }
    }
}

/// Layer D — Material3 IconButton + vector Icon (built inline so we
/// don't need compose-material-icons-extended on wasi). Identical
/// structure to Material3 DatePicker's chevron. If this crashes →
/// bug is in vector-icon rasterization (DrawCache → bitmap-canvas
/// snapshot → drawImage path for an icon that re-rasterizes per
/// state change).
private val chevronLeft: ImageVector = ImageVector.Builder(
    name = "ChevronLeft", defaultWidth = 24.dp, defaultHeight = 24.dp,
    viewportWidth = 24f, viewportHeight = 24f,
).apply {
    path(fill = SolidColor(Color.Black)) {
        moveTo(15.41f, 16.59f)
        lineTo(10.83f, 12f)
        lineToRelative(4.58f, -4.59f)
        lineTo(14f, 6f)
        lineToRelative(-6f, 6f)
        lineToRelative(6f, 6f)
        close()
    }
}.build()

private val chevronRight: ImageVector = ImageVector.Builder(
    name = "ChevronRight", defaultWidth = 24.dp, defaultHeight = 24.dp,
    viewportWidth = 24f, viewportHeight = 24f,
).apply {
    path(fill = SolidColor(Color.Black)) {
        moveTo(8.59f, 16.59f)
        lineTo(13.17f, 12f)
        lineTo(8.59f, 7.41f)
        lineTo(10f, 6f)
        lineToRelative(6f, 6f)
        lineToRelative(-6f, 6f)
        close()
    }
}.build()

@Composable
private fun LayerDVectorIconButton() {
    var month by remember { mutableIntStateOf(0) }
    Text("Layer D — + ImageVector icons", style = MaterialTheme.typography.labelMedium)
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        IconButton(onClick = { month-- }) {
            Icon(imageVector = chevronLeft, contentDescription = "prev")
        }
        AnimatedContent(targetState = month, label = "month") { m ->
            Text("month = $m", modifier = Modifier.padding(horizontal = 8.dp))
        }
        IconButton(onClick = { month++ }) {
            Icon(imageVector = chevronRight, contentDescription = "next")
        }
    }
}
