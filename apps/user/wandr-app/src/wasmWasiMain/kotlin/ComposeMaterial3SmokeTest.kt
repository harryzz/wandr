package testapp

import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import org.jetbrains.skiko.wasi.wit.Canvas as WitCanvas

/**
 * compose-material3 smoke test. Compiles iff compose-material3-wasi is
 * linkable. Touches the foundational Material3 design-token surface:
 *   - lightColorScheme / darkColorScheme factories
 *   - ColorScheme, Typography, Shapes
 *   - MaterialTheme companion (provides defaults)
 *
 * Doesn't exercise @Composable popups (DropdownMenu / AlertDialog / etc.) —
 * those route through ComposeSceneLayer (Option A in-canvas overlay per §11)
 * and need a real composition.
 */
fun composeMaterial3SmokeTest() {
    val light = lightColorScheme()
    val dark  = darkColorScheme()
    val custom = lightColorScheme(
        primary = Color(0xFF6750A4),
        onPrimary = Color.White,
        secondary = Color(0xFF625B71),
    )
    val schemeRef: ColorScheme = light
    val typographyRef: Typography = MaterialTheme::class.let { Typography() }
    val shapesRef: Shapes = Shapes()

    WitCanvas.Import.logMessage(
        "compose-material3 smoke: " +
        "lightColorScheme=${schemeRef::class.simpleName}, " +
        "light.primary=${light.primary}, dark.primary=${dark.primary}, " +
        "custom.primary=${custom.primary}, custom.secondary=${custom.secondary}, " +
        "typography=${typographyRef::class.simpleName}, " +
        "shapes={small=${shapesRef.small::class.simpleName}, medium=${shapesRef.medium::class.simpleName}, large=${shapesRef.large::class.simpleName}}, " +
        "MaterialTheme-companion-resolved=${MaterialTheme::class.simpleName}"
    )
}
