// Second cross-app dep demo (task 40). Calls `wandr:emoji/picker.list-all`
// via the new generic wandr-host dep wiring (task 39), renders the
// result as a category-grouped grid of emojis.
//
// Side-purpose: visual proof that wandr-host's `wire_dep_into_linker`
// works for arbitrary system components, not just markdown. The
// markdown_bindings module + per-dep match arm in wandr-host are gone
// — both this card AND MarkdownCard load through the same generic
// introspection-based path.

@file:OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)

package testapp

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.jetbrains.skiko.wasi.wit.Canvas as WitCanvas
import testapp.emoji.Emoji
import testapp.emoji.listAllEmojis

@Composable
internal fun EmojiCard() {
    val grouped = remember { loadGrouped() }
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = "Task 40 — cross-app dep (emoji)",
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (grouped.isEmpty()) {
                Text(
                    text = "list-all() returned empty — see logcat",
                    color = MaterialTheme.colorScheme.error,
                    fontSize = 12.sp,
                )
            } else {
                grouped.forEach { (category, emojis) ->
                    Text(
                        text = category,
                        fontSize = 11.sp,
                        color = MaterialTheme.colorScheme.outline,
                    )
                    FlowRow(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(4.dp),
                        verticalArrangement = Arrangement.spacedBy(4.dp),
                    ) {
                        emojis.forEach { e ->
                            Text(text = e.glyph, fontSize = 22.sp)
                        }
                    }
                }
            }
        }
    }
}

/// Loads + groups by category preserving the dep's iteration order
/// (which is already grouped). Returns ordered list of (category,
/// emojis-in-category) pairs.
private fun loadGrouped(): List<Pair<String, List<Emoji>>> = try {
    val all = listAllEmojis()
    WitCanvas.Import.logMessage("emoji-card: list-all() → ${all.size} emojis")
    val orderedKeys = LinkedHashSet<String>()
    val byKey = HashMap<String, MutableList<Emoji>>()
    for (e in all) {
        if (orderedKeys.add(e.category)) byKey[e.category] = mutableListOf()
        byKey[e.category]!!.add(e)
    }
    orderedKeys.map { it to byKey[it]!! }
} catch (t: Throwable) {
    WitCanvas.Import.logMessage("emoji-card: list-all() FAILED: ${t.message ?: t::class.simpleName}")
    emptyList()
}
