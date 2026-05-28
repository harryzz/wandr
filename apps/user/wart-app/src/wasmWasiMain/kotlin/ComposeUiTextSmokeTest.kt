package testapp

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.ParagraphStyle
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontSynthesis
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.intl.Locale
import androidx.compose.ui.text.intl.LocaleList
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.sp
import org.jetbrains.skiko.wasi.wit.Canvas as WitCanvas

/**
 * Smoke test for compose-ui-text-wasi. Compiles iff the publication is usable;
 * exercises upstream `AnnotatedString`/`TextStyle`/`SpanStyle`/`Locale`/etc.
 * API. Locale.current routes through our wasi:android-locale WIT (returns
 * bg-BG on this device).
 */
fun composeUiTextSmokeTest() {
    // ── Locale: pulls user's primary via host WIT ───────────────────────────
    val current = Locale.current
    val custom  = Locale("ja-JP")
    val list    = LocaleList(listOf(current, custom))

    // ── AnnotatedString with mixed spans ────────────────────────────────────
    val annotated = AnnotatedString.Builder().apply {
        append("Hello, ")
        withStyle(SpanStyle(color = Color.Red, fontWeight = FontWeight.Bold)) {
            append("world")
        }
        append("! ")
        withStyle(SpanStyle(
            color = Color.Blue,
            fontStyle = FontStyle.Italic,
            textDecoration = TextDecoration.Underline,
            letterSpacing = 0.5.sp,
        )) {
            append("italic underlined")
        }
    }.toAnnotatedString()

    // ── TextStyle composition ──────────────────────────────────────────────
    val baseStyle = TextStyle(
        color = Color(0xFF1A1A2E.toInt()),
        fontSize = 14.sp,
        fontFamily = FontFamily.SansSerif,
        fontWeight = FontWeight.Normal,
        fontSynthesis = FontSynthesis.All,
        textAlign = TextAlign.Start,
    )
    val merged = baseStyle.merge(SpanStyle(color = Color.Green, fontWeight = FontWeight.Bold))

    // ── ParagraphStyle ─────────────────────────────────────────────────────
    val paragraphStyle = ParagraphStyle(
        textAlign = TextAlign.Justify,
        lineHeight = 20.sp,
    )

    // ── TextRange ──────────────────────────────────────────────────────────
    val range1 = TextRange(0, 5)
    val range2 = TextRange(7, 12)
    val rangeCollapsed = TextRange(3)

    WitCanvas.Import.logMessage(
        "compose-ui-text smoke: " +
        "Locale.current=${current.toLanguageTag()} (lang=${current.language} region=${current.region}), " +
        "custom=${custom.toLanguageTag()}, list.size=${list.size}, " +
        "annotated.length=${annotated.length} text=\"${annotated.text}\" spans=${annotated.spanStyles.size}, " +
        "baseStyle.color=${baseStyle.color} merged.color=${merged.color} merged.weight=${merged.fontWeight}, " +
        "paragraph.align=${paragraphStyle.textAlign} lineHeight=${paragraphStyle.lineHeight}, " +
        "ranges=[${range1}, ${range2}, collapsed=${rangeCollapsed} (collapsed=${rangeCollapsed.collapsed})]"
    )
}
