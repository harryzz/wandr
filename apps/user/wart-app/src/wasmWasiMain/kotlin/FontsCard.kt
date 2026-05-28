// Third cross-app dep demo (task 41). Calls `war:fonts/loader.list-all`
// to enumerate the device's /system/fonts/ directory (exposed to the
// dep via a WASI preopen at /system-fonts) and shows a count + a few
// sample family names. The actual font-rendering improvement happens
// in MarkdownCard via the host's family-alias path mapping — this
// card is the validator that the dep itself works.

package testapp

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.jetbrains.skiko.wasi.wit.Canvas as WitCanvas
import testapp.fonts.FontInfo
import testapp.fonts.listAllFonts

@Composable
internal fun FontsCard() {
    val fonts = remember { loadFonts() }
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(
                text = "Task 41 — cross-app dep (system fonts)",
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (fonts.isEmpty()) {
                Text(
                    text = "list-all() returned empty — preopen missing or read failed",
                    color = MaterialTheme.colorScheme.error,
                    fontSize = 12.sp,
                )
            } else {
                // Distinct families, ordered as the dep returned them.
                val families = fonts.map { it.family }.distinct()
                Text(
                    text = "${fonts.size} files, ${families.size} families on /system/fonts/",
                    fontSize = 12.sp,
                )
                Text(
                    text = "Sample (rendered in default font here): " +
                        families.take(8).joinToString(", "),
                    fontSize = 11.sp,
                    color = MaterialTheme.colorScheme.outline,
                )
                // The point: render samples in their own fonts, to
                // show that the host's family-alias mapping kicks in.
                Text(
                    text = "Serif sample: The quick brown fox.",
                    fontFamily = FontFamily.Serif,
                    fontSize = 13.sp,
                )
                Text(
                    text = "Monospace sample: fn main() { println!(\"hi\"); }",
                    fontFamily = FontFamily.Monospace,
                    fontSize = 12.sp,
                )
            }
        }
    }
}

private fun loadFonts(): List<FontInfo> = try {
    val all = listAllFonts()
    WitCanvas.Import.logMessage(
        "fonts-card: list-all() → ${all.size} font files; " +
        "${all.map { it.family }.distinct().size} distinct families"
    )
    all
} catch (t: Throwable) {
    WitCanvas.Import.logMessage("fonts-card: list-all() FAILED: ${t.message ?: t::class.simpleName}")
    emptyList()
}
