// wandr.ime.keyboard — first-party IME app for the wandr runtime.
// Renders a full keyboard (English QWERTY + Symbols + Symbols2 +
// Emoji + editor-driven Numeric/Phone/Email/Url/Password, all with
// shift / layout-cycle / etc) as a bottom-strip overlay surface.
// Additional languages (Bulgarian, French, …) load at startup from
// `wandr.lang.*` plugins via `wandr:keyboard-lang/lang` (task 49). Pushes keystrokes to the
// currently-focused editor in another wandr guest via the new IME
// WIT contract:
//   Keyboard.Import.sendKeyEvent(code-point, key-id, action)
//     → wandr-host keyboard_host_impl (UNIX socket)
//       → arbiter cmd_ime_route ("ime-send-key-event")
//         → focused-pid's per-host control socket
//           → ime_inbound queue → render-loop drain
//             → dispatch_key_v2 → Compose KeyEvent
//
// All keyboard layout / shift / layout-switch logic lives in
// ImeKeyboard.kt — a port of wandr-app's WasiSoftKeyboard.kt
// adapted to call the WIT verb directly instead of a callback.
//
// This file is intentionally thin: scene setup + overlay-height
// request + composable root. See tasks/47-ime-via-guest-app.md
// step 3c.

@file:OptIn(androidx.compose.ui.InternalComposeUiApi::class)

package testapp

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.PlatformContext
import androidx.compose.ui.platform.WasiFrameDispatcher
import androidx.compose.ui.platform.WindowInfo
import androidx.compose.ui.scene.CanvasLayersComposeScene
import androidx.compose.ui.scene.ComposeScene
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.IntSize
import wandr.platform.KeyboardSend as WitKeyboard
import wandr.platform.Display as WitDisplay
import org.jetbrains.skiko.wasi.shell.Metrics as WitWindow

val wasiFrameDispatcher: WasiFrameDispatcher = WasiFrameDispatcher()

/// Task 64 — the live scene, exposed so the `frame-pacing` export
/// (`RendererExports.kt`) can read `hasInvalidations()`. Set by
/// `buildRealComposeScene`.
var realScenePacing: ComposeScene? = null

/// Task 64 — milliseconds until the next frame Compose wants, for the host's
/// on-demand render gate. 0 if the scene has pending recomposition / draws /
/// frame-clock awaiters; else the nearest `WasiFrameDispatcher` deadline (a
/// pending `delay()` / cursor blink — `flush()` is their only heartbeat, so
/// they must be included); else a large idle value (host clamps to its cap).
fun nextFrameDelayMillis(): Int {
    val scene = realScenePacing ?: return 0
    if (scene.hasInvalidations()) return 0
    val now = androidx.compose.ui.cachedNanoTime() / 1_000_000L
    val d = wasiFrameDispatcher.nextDeadlineMillis(now)
    val idle = 100_000
    return when {
        d >= idle -> idle
        d < 0L -> 0
        else -> d.toInt()
    }
}

// Kept as no-op stubs so Main.kt's setup mirrors wandr-app verbatim
// (both apps share Main.kt). The IME doesn't need a soft-keyboard
// bridge — IT IS the soft keyboard.
var wasiSoftKeyboardKeyHandler: (androidx.compose.ui.input.key.KeyEvent) -> Unit = {}
var wasiHideKeyboardRequest: () -> Unit = {}

/// Mutable [WindowInfo] so the renderer (Main.kt) can update
/// `containerSize` on a runtime surface-size change — task 62 overlay
/// rotation (the IME's bottom strip flips to a side strip in landscape,
/// swapping logical dims). Mirrors wandr-app's MutableSceneWindowInfo.
class MutableSceneWindowInfo(initial: IntSize) : WindowInfo {
    override var isWindowFocused: Boolean = true
    override var keyboardModifiers: androidx.compose.ui.input.pointer.PointerKeyboardModifiers =
        androidx.compose.ui.input.pointer.PointerKeyboardModifiers(0)
    // Backed by snapshot state so a composable that reads it (KeyboardScreen)
    // recomposes when the host swaps logical dims on rotation — that's the
    // signal the IME uses to re-request its per-orientation overlay height.
    override var containerSize: IntSize by mutableStateOf(initial)
}

/// The live scene's WindowInfo, set by `buildRealComposeScene`, so the
/// render delegate can update `containerSize` when the host swaps logical
/// dimensions on rotation.
var realSceneWindowInfo: MutableSceneWindowInfo? = null

fun buildRealComposeScene(widthPx: Int, heightPx: Int, density: Float): ComposeScene {
    val sceneWindowInfo = MutableSceneWindowInfo(IntSize(widthPx, heightPx))
    realSceneWindowInfo = sceneWindowInfo
    val platformContext = object : PlatformContext by PlatformContext.Empty() {
        override val windowInfo: WindowInfo = sceneWindowInfo
    }
    val scene = CanvasLayersComposeScene(
        density = Density(density),
        size = IntSize(widthPx, heightPx),
        coroutineContext = wasiFrameDispatcher,
        platformContext = platformContext,
    )
    scene.setContent {
        MaterialTheme(colorScheme = darkColorScheme()) {
            KeyboardScreen()
        }
    }
    realScenePacing = scene // task 64 — frame-pacing export reads hasInvalidations()
    return scene
}

// ── Keyboard-height policy (task 71 step 1 — intrinsic dp sizing) ──────────
// The IME OWNS its size and derives it — NO hardcoded pixels, NO per-orientation
// constants, resolution/density-independent. Height = (rows of the tallest
// layout) × a comfortable row height, in dp, converted to px via the reported
// density, capped to a fraction of the REAL screen so it never starves content.
// The host applies the result verbatim (dumb applier). These three are the only
// tunables — each a justified single source of truth, in the layer (the IME)
// that owns keyboard-size policy.
//
/** Comfortable key-row height. Material's minimum touch target is 48 dp. */
private const val ROW_HEIGHT_DP: Float = 48f
/** Gap around + between rows — mirrors `ImeKeyboard`'s Column padding + spacing
 *  (4 dp), so the requested surface matches what the layout actually draws. */
private const val ROW_GAP_DP: Float = 4f
/** Ceiling: the keyboard never occludes more than this fraction of the screen's
 *  current-orientation height (protects the focused content). Only bites in
 *  landscape, where the short edge makes the intrinsic dp height too tall:
 *  portrait intrinsic (~32% of the long edge) stays under it; landscape caps to
 *  this fraction of the short edge (~45%). */
private const val MAX_SCREEN_FRACTION: Float = 0.45f

@Composable
private fun KeyboardScreen() {
    // The tallest layout's row count drives the surface size, so the surface is
    // stable across layout switches (a 4-row symbol layout just has whitespace —
    // no per-keystroke SF resize). Derived from the loaded layouts, not a literal.
    val maxRows = remember { ImeKeyboardDefaults.loadAllLayouts().maxOf { it.rows.size } }

    // Re-request our overlay height whenever the surface geometry changes
    // (startup + every rotation). `containerSize` is snapshot-backed, so this
    // recomposes on rotation; we read the REAL panel size from the `display`
    // namespace (an overlay's own surface is just a strip — it can't tell us)
    // and the density from `window`, then derive the height. No orientation
    // branch: the same formula on the rotated `display-size` gives the right
    // result for free.
    val containerSize = LocalWindowInfo.current.containerSize
    LaunchedEffect(containerSize) {
        val density = WitWindow.Import.getDensity()                 // px per dp
        val rows = maxRows.toFloat()
        val intrinsicDp = rows * ROW_HEIGHT_DP + (rows + 1f) * ROW_GAP_DP
        val intrinsicPx = (intrinsicDp * density).toInt()
        val screenH = WitDisplay.Import.displaySize().height.toFloat() // real panel, current orient
        val capPx = (screenH * MAX_SCREEN_FRACTION).toInt()
        val heightPx = minOf(intrinsicPx, capPx).coerceAtLeast(1)
        WitKeyboard.Import.requestOverlayHeight(heightPx.toUInt())
    }

    // The overlay surface IS the keyboard panel — fillMaxSize uses
    // the whole surface (sized by the host via setSize on
    // request-overlay-height). The dark base background shows
    // through any whitespace between the keys / above shorter
    // layouts.
    Box(
        modifier = Modifier.fillMaxSize().background(Color(0xFF1F1F1F)),
        contentAlignment = Alignment.BottomCenter,
    ) {
        ImeKeyboard()
    }
}
