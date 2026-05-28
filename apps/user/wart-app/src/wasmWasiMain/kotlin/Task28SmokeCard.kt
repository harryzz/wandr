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
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

/// Task-28 smoke verification. One-shot at composition time: builds a
/// 32×32 bitmap-backed Canvas and exercises every `bc-*` method exposed
/// by the Path D wiring. Each call sits inside a runCatching so a
/// signature mismatch surfaces as a status line rather than a SIGILL.
/// The on-screen output names each method and "ok" / the thrown message
/// — quick scan tells us if a verb is still broken.
@Composable
internal fun Task28SmokeCard() {
    val status = remember { buildStatus() }
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Text(
                text = "Task 28 — bitmap-canvas (Path D)",
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(text = status, fontSize = 10.sp)
        }
    }
}

private fun buildStatus(): String {
    val lines = mutableListOf<String>()
    fun row(name: String, body: () -> Unit) {
        try { body(); lines += "$name: ok" }
        catch (t: Throwable) { lines += "$name: FAIL ${t.message?.take(60)}" }
    }
    // Construct bitmap and canvas. allocPixels captures the dimensions
    // so Canvas(bitmap) sizes the host surface correctly.
    val bitmap = org.jetbrains.skia.Bitmap().apply {
        allocPixels(org.jetbrains.skia.ImageInfo.makeN32Premul(32, 32))
    }
    val canvas = org.jetbrains.skia.Canvas(bitmap)
    val paint = org.jetbrains.skia.Paint().also { it.color = 0xFF00AAFF.toInt() }
    val rect = org.jetbrains.skia.Rect(2f, 2f, 30f, 30f)
    val rrect = org.jetbrains.skia.RRect(0f, 0f, 32f, 32f, floatArrayOf(4f))
    val rrectInner = org.jetbrains.skia.RRect(8f, 8f, 24f, 24f, floatArrayOf(2f))
    val path = org.jetbrains.skia.Path().moveTo(0f, 0f).lineTo(32f, 32f)

    row("save")             { canvas.save() }
    row("translate")        { canvas.translate(1f, 1f) }
    row("scale")            { canvas.scale(1.1f, 1.1f) }
    row("rotate")           { canvas.rotate(5f, 16f, 16f) }
    row("skew")             { canvas.skew(0.1f, 0f) }
    row("concat")           { canvas.concat(org.jetbrains.skia.Matrix33(1f,0f,0f,0f,1f,0f,0f,0f,1f)) }
    row("setMatrix")        { canvas.setMatrix(org.jetbrains.skia.Matrix33(1f,0f,0f,0f,1f,0f,0f,0f,1f)) }
    row("resetMatrix")      { canvas.resetMatrix() }
    row("clipRect")         { canvas.clipRect(rect, org.jetbrains.skia.ClipMode.INTERSECT, true) }
    row("clipRRect")        { canvas.clipRRect(rrect, org.jetbrains.skia.ClipMode.INTERSECT, true) }
    row("clipPath")         { canvas.clipPath(path, org.jetbrains.skia.ClipMode.INTERSECT, true) }
    row("clear")            { canvas.clear(0xFF202020.toInt()) }
    row("drawPaint")        { canvas.drawPaint(paint) }
    row("drawRect")         { canvas.drawRect(rect, paint) }
    row("drawRRect")        { canvas.drawRRect(rrect, paint) }
    row("drawOval")         { canvas.drawOval(rect, paint) }
    row("drawCircle")       { canvas.drawCircle(16f, 16f, 10f, paint) }
    row("drawLine")         { canvas.drawLine(0f, 0f, 32f, 32f, paint) }
    row("drawArc")          { canvas.drawArc(rect, 0f, 90f, true, paint) }
    row("drawDRRect")       { canvas.drawDRRect(rrect, rrectInner, paint) }
    row("drawPath")         { canvas.drawPath(path, paint) }
    row("drawPoint")        { canvas.drawPoint(8f, 8f, paint) }
    row("drawPoints")       { canvas.drawPoints(floatArrayOf(2f,2f,30f,30f), paint) }
    row("drawLines")        { canvas.drawLines(floatArrayOf(2f,2f,30f,30f), paint) }
    row("drawPolygon")      { canvas.drawPolygon(floatArrayOf(2f,2f,30f,2f,30f,30f,2f,30f), paint) }
    row("drawString")       { canvas.drawString("x", 8f, 24f, null, paint) }
    row("drawVertices") {
        canvas.drawVertices(
            org.jetbrains.skia.VertexMode.TRIANGLES,
            floatArrayOf(0f,0f, 32f,0f, 16f,32f),
            null, null, null,
            org.jetbrains.skia.BlendMode.SRC_OVER, paint)
    }
    row("saveLayer")        { canvas.saveLayer(rect, paint) }
    row("restore")          { canvas.restore() }
    row("restoreToCount")   { canvas.restoreToCount(0) }
    // Snapshot: lifts the host surface pixels into an Image. The Image
    // can now flow through WasiCanvas.drawImage on the main canvas.
    row("snapshot→Image") {
        val img = org.jetbrains.skia.Image.makeFromBitmap(bitmap)
        // Can't peek at internal id from app code — width!=0 implies the
        // snapshot returned a sized image (the prior id=0 sentinel
        // returned a 0×0 placeholder).
        check(img.width > 0 && img.height > 0) { "snapshot returned ${img.width}×${img.height}" }
        img.close()
    }
    canvas.close()
    return lines.joinToString("\n")
}
