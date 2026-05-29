// Task 57 — launcher grid. Lists installed apps (via the new
// `my:skiko-gfx/launcher` host verb → install-dir scan) as letter-tile
// icons; tapping a tile asks the arbiter to launch/foreground that app.
// When wart-app is the arbiter's designated home (`set-home
// com.example.wart-app`), this card is the home/launcher experience.
//
// v1: letter-tile icons (first label char on a hash-colored tile — no
// per-app art) + a theme-gradient backdrop. A dedicated `war.launcher`
// warpkg with a richer full-screen grid is the follow-up.

@file:OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)

package testapp

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.jetbrains.skiko.wasi.wit.Canvas as WitCanvas
import testapp.launcher.AppEntry
import testapp.launcher.launchApp
import testapp.launcher.listApps

@Composable
internal fun LauncherCard() {
    val apps = remember { loadApps() }
    // Theme-gradient backdrop (the "wallpaper" for v1 — reuses the
    // Material scheme rather than a bundled image).
    val gradient = Brush.verticalGradient(
        listOf(
            MaterialTheme.colorScheme.primaryContainer,
            MaterialTheme.colorScheme.surfaceVariant,
        )
    )
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = Color.Transparent),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .background(gradient)
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Text(
                text = "Apps",
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onPrimaryContainer,
            )
            if (apps.isEmpty()) {
                Text(
                    text = "No installed apps found (see logcat).",
                    fontSize = 12.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                FlowRow(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(16.dp),
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    apps.forEach { app -> AppTile(app) }
                }
            }
        }
    }
}

@Composable
private fun AppTile(app: AppEntry) {
    Column(
        modifier = Modifier
            .width(72.dp)
            .clip(RoundedCornerShape(12.dp))
            .clickable {
                WitCanvas.Import.logMessage("launcher-card: tap → launch ${app.appId}")
                launchApp(app.appId)
            }
            .padding(4.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        // Letter tile — first label char on a hash-derived color.
        Box(
            modifier = Modifier
                .size(56.dp)
                .clip(RoundedCornerShape(14.dp))
                .background(tileColor(app.appId)),
            contentAlignment = Alignment.Center,
        ) {
            Text(
                text = app.label.trim().take(1).uppercase().ifEmpty { "?" },
                fontSize = 26.sp,
                color = Color.White,
            )
        }
        Text(
            text = app.label,
            fontSize = 11.sp,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            textAlign = TextAlign.Center,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/// Deterministic tile color from the app-id (stable across launches).
private fun tileColor(appId: String): Color {
    val palette = listOf(
        0xFF4285F4, 0xFFEA4335, 0xFFFBBC05, 0xFF34A853,
        0xFFAB47BC, 0xFF00ACC1, 0xFFFF7043, 0xFF5C6BC0,
    )
    var h = 0
    for (c in appId) h = h * 31 + c.code
    return Color(palette[((h % palette.size) + palette.size) % palette.size])
}

private fun loadApps(): List<AppEntry> = try {
    val apps = listApps()
    WitCanvas.Import.logMessage("launcher-card: list-apps → ${apps.size} app(s)")
    apps
} catch (t: Throwable) {
    WitCanvas.Import.logMessage("launcher-card: list-apps FAILED: ${t.message ?: t::class.simpleName}")
    emptyList()
}
