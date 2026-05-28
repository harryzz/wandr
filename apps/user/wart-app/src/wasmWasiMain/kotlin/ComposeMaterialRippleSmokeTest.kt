package testapp

import androidx.compose.material.ripple.RippleAlpha
import androidx.compose.material.ripple.createRippleModifierNode
import org.jetbrains.skiko.wasi.wit.Canvas as WitCanvas

/**
 * compose-material-ripple smoke test. Compiles iff
 * compose-material-ripple-wasi is linkable. Uses only the non-deprecated
 * surface: RippleAlpha + createRippleModifierNode. RippleTheme /
 * LocalRippleTheme / rememberRipple are all deprecated in favour of the
 * Indication-based ripple — material/material3 ports will exercise the new
 * surface.
 */
fun composeMaterialRippleSmokeTest() {
    val customAlpha = RippleAlpha(
        draggedAlpha = 0.16f,
        focusedAlpha = 0.12f,
        hoveredAlpha = 0.08f,
        pressedAlpha = 0.24f,
    )
    val defaultAlpha = RippleAlpha(
        draggedAlpha = 0.0f,
        focusedAlpha = 0.0f,
        hoveredAlpha = 0.0f,
        pressedAlpha = 0.0f,
    )
    val createFactory = ::createRippleModifierNode

    WitCanvas.Import.logMessage(
        "compose-material-ripple smoke: " +
        "alpha={pressed=${customAlpha.pressedAlpha}, focused=${customAlpha.focusedAlpha}, " +
        "dragged=${customAlpha.draggedAlpha}, hovered=${customAlpha.hoveredAlpha}}, " +
        "default-alpha-zero=${defaultAlpha.pressedAlpha}, " +
        "createRippleModifierNode-resolved=${createFactory != null}"
    )
}
