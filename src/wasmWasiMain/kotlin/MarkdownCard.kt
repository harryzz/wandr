// MarkdownCard v2 — proper Compose rendering of the document tree
// returned by the cross-app `war:markdown/renderer` dep. Block variants
// map to real Compose widgets: paragraph + heading → AnnotatedString
// Text with FontWeight/FontStyle/FontFamily applied per inline-style;
// code-block → monospace Surface; bullet-list / ordered-list → Column
// of "• " / "N. " prefixed rows; block-quote → indented Box with a
// left vertical bar; thematic-break → HorizontalDivider.
//
// Task 36 follow-up — see tasks/36-cross-app-deps.md "Deferred"
// section + docs/architecture-host-guest-boundary.md.

package testapp

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.jetbrains.skiko.wasi.wit.Canvas as WitCanvas
import testapp.markdown.Block
import testapp.markdown.Document
import testapp.markdown.InlineStyle
import testapp.markdown.MdListItem
import testapp.markdown.Run
import testapp.markdown.SimpleBlock
import testapp.markdown.renderDocument

private const val SOURCE = """
# Cross-app dep demo

This text was parsed by **markdown-renderer**, a *separate* WASM
component installed under `/data/.../system-apps/war.markdown.renderer/`,
running in the **same** Store as this Compose UI.

## How it works

The cross-app dep linker proxy lives in `wart-host/src/app_loader.rs`
(`wire_markdown_dep`). When this card composes, it calls `render()`
through the proxy — into the markdown component — which returns the
parsed `document` tree back into wart-app's linear memory.

- One block per `LazyColumn` row
- Inline runs carry stacked styles
- `option<string>` link-url for hyperlinks

1. Install the dep (system bundle).
2. Install the consumer with `[dependencies.markdown]`.
3. Run `wart-host --standalone --app com.example.wart-app`.

> Cross-app deps now work for **both** CLI (`md-smoke-rust`) and
> Compose (this card) consumers — same `wire_markdown_dep` proxy,
> different consumer shape.

```rust
fn render(source: String) -> Document {
    // pulldown-cmark → WIT records
}
```

---

That horizontal rule above came from a thematic break.
"""

@Composable
internal fun MarkdownCard() {
    val doc = remember { renderOnce() }
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Text(
                text = "Task 36 — cross-app dep (markdown)",
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (doc == null) {
                Text(
                    text = "render() failed — see logcat for stack",
                    color = MaterialTheme.colorScheme.error,
                    fontSize = 12.sp,
                )
            } else {
                doc.blocks.forEach { RenderBlock(it) }
            }
        }
    }
}

private fun renderOnce(): Document? = try {
    val d = renderDocument(SOURCE)
    WitCanvas.Import.logMessage("markdown-card: render() → ${d.blocks.size} blocks (full tree lifted)")
    d
} catch (t: Throwable) {
    WitCanvas.Import.logMessage("markdown-card: render() FAILED: ${t.message ?: t::class.simpleName}")
    null
}

// ── Block-level renderers ────────────────────────────────────────────

@Composable
private fun RenderBlock(block: Block) {
    when (block) {
        is Block.Paragraph    -> RenderRuns(block.runs)
        is Block.Heading      -> RenderHeading(block.level, block.runs)
        is Block.CodeBlock    -> RenderCodeBlock(block.language, block.text)
        is Block.BulletList   -> RenderBulletList(block.items)
        is Block.OrderedList  -> RenderOrderedList(block.start, block.items)
        is Block.BlockQuote   -> RenderBlockQuote(block.blocks)
        Block.ThematicBreak   -> HorizontalDivider(modifier = Modifier.padding(vertical = 4.dp))
    }
}

@Composable
private fun RenderSimpleBlock(block: SimpleBlock) {
    when (block) {
        is SimpleBlock.Paragraph  -> RenderRuns(block.runs)
        is SimpleBlock.Heading    -> RenderHeading(block.level, block.runs)
        is SimpleBlock.CodeBlock  -> RenderCodeBlock(block.language, block.text)
        SimpleBlock.ThematicBreak -> HorizontalDivider(modifier = Modifier.padding(vertical = 4.dp))
    }
}

@Composable
private fun RenderRuns(runs: List<Run>) {
    Text(text = runsToAnnotated(runs), fontSize = 13.sp)
}

@Composable
private fun RenderHeading(level: Int, runs: List<Run>) {
    val size = when (level) {
        1 -> 22.sp
        2 -> 18.sp
        3 -> 16.sp
        else -> 14.sp
    }
    Text(
        text = runsToAnnotated(runs),
        fontSize = size,
        fontWeight = FontWeight.Bold,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(top = 4.dp),
    )
}

@Composable
private fun RenderCodeBlock(language: String?, text: String) {
    Surface(
        color = MaterialTheme.colorScheme.surface,
        shape = RoundedCornerShape(4.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(modifier = Modifier.padding(8.dp)) {
            if (!language.isNullOrEmpty()) {
                Text(
                    text = language,
                    fontSize = 9.sp,
                    color = MaterialTheme.colorScheme.outline,
                )
            }
            Text(
                text = text.trimEnd(),
                fontFamily = FontFamily.Monospace,
                fontSize = 11.sp,
                color = MaterialTheme.colorScheme.onSurface,
            )
        }
    }
}

@Composable
private fun RenderBulletList(items: List<MdListItem>) {
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        items.forEach { item ->
            Row(verticalAlignment = Alignment.Top) {
                Text(text = "•  ", fontSize = 13.sp)
                Column { item.blocks.forEach { RenderSimpleBlock(it) } }
            }
        }
    }
}

@Composable
private fun RenderOrderedList(start: Int, items: List<MdListItem>) {
    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
        items.forEachIndexed { idx, item ->
            Row(verticalAlignment = Alignment.Top) {
                Text(text = "${start + idx}.  ", fontSize = 13.sp)
                Column { item.blocks.forEach { RenderSimpleBlock(it) } }
            }
        }
    }
}

@Composable
private fun RenderBlockQuote(blocks: List<SimpleBlock>) {
    Row(modifier = Modifier.height(IntrinsicSize.Min)) {
        Box(
            modifier = Modifier
                .width(3.dp)
                .fillMaxHeight()
                .background(MaterialTheme.colorScheme.outline),
        )
        Spacer(modifier = Modifier.width(8.dp))
        Column(
            verticalArrangement = Arrangement.spacedBy(4.dp),
            modifier = Modifier.padding(vertical = 2.dp),
        ) {
            blocks.forEach { RenderSimpleBlock(it) }
        }
    }
}

// ── Inline-style → AnnotatedString ───────────────────────────────────

private fun runsToAnnotated(runs: List<Run>): AnnotatedString = buildAnnotatedString {
    for (run in runs) {
        val isCode = InlineStyle.Code in run.styles
        val isStrong = InlineStyle.Strong in run.styles
        val isEmphasis = InlineStyle.Emphasis in run.styles
        val isLink = run.linkUrl != null
        val style = SpanStyle(
            fontWeight = if (isStrong) FontWeight.Bold else FontWeight.Normal,
            fontStyle = if (isEmphasis) FontStyle.Italic else FontStyle.Normal,
            fontFamily = if (isCode) FontFamily.Monospace else FontFamily.Default,
            background = if (isCode) Color(0x33000000) else Color.Unspecified,
            color = if (isLink) Color(0xFF7BBEFF.toInt()) else Color.Unspecified,
            textDecoration = if (isLink) TextDecoration.Underline else TextDecoration.None,
        )
        withStyle(style) { append(run.text) }
    }
}
