// war.ime.keyboard — first-party IME app for the wart runtime.
// Renders a full keyboard (English QWERTY + Symbols + Symbols2 +
// Emoji + editor-driven Numeric/Phone/Email/Url/Password, all with
// shift / layout-cycle / etc) as a bottom-strip overlay surface.
// Additional languages (Bulgarian, French, …) load at startup from
// `war.lang.*` plugins via `war:keyboard-lang/lang` (task 49). Pushes keystrokes to the
// currently-focused editor in another wart guest via the new IME
// WIT contract:
//   Keyboard.Import.sendKeyEvent(code-point, key-id, action)
//     → wart-host keyboard_host_impl (UNIX socket)
//       → arbiter cmd_ime_route ("ime-send-key-event")
//         → focused-pid's per-host control socket
//           → ime_inbound queue → render-loop drain
//             → dispatch_key_v2 → Compose KeyEvent
//
// All keyboard layout / shift / layout-switch logic lives in
// ImeKeyboard.kt — a port of wart-app's WasiSoftKeyboard.kt
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
import org.jetbrains.skiko.wasi.wit.Keyboard as WitKeyboard

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

// Kept as no-op stubs so Main.kt's setup mirrors wart-app verbatim
// (both apps share Main.kt). The IME doesn't need a soft-keyboard
// bridge — IT IS the soft keyboard.
var wasiSoftKeyboardKeyHandler: (androidx.compose.ui.input.key.KeyEvent) -> Unit = {}
var wasiHideKeyboardRequest: () -> Unit = {}

/// Mutable [WindowInfo] so the renderer (Main.kt) can update
/// `containerSize` on a runtime surface-size change — task 62 overlay
/// rotation (the IME's bottom strip flips to a side strip in landscape,
/// swapping logical dims). Mirrors wart-app's MutableSceneWindowInfo.
class MutableSceneWindowInfo(initial: IntSize) : WindowInfo {
    override var isWindowFocused: Boolean = true
    override var keyboardModifiers: androidx.compose.ui.input.pointer.PointerKeyboardModifiers =
        androidx.compose.ui.input.pointer.PointerKeyboardModifiers(0)
    override var containerSize: IntSize = initial
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

/// Task 47 step 3c — preferred panel height in physical pixels. The
/// host's wart-host receives this via `Keyboard.Import.requestOverlayHeight`,
/// forwards to `sf_resize_overlay`, and flushes the ANativeWindow's
/// buffer geometry so the next frame draws to the new size. The IME
/// is launched with an `INITIAL_OVERLAY_PX=1200` surface; this verb
/// trims it to a sensible keyboard height (~38% of a 2880-px panel).
///
/// Sized for the 5-row English / language-plugin layouts (digits + 3
/// letter rows + modifier row). Symbols / Emoji are 4 rows so they
/// just have a bit more whitespace at the top — harmless. If a
/// future layout grows, this can be re-requested on layout change.
private const val OVERLAY_HEIGHT_PX: UInt = 1200u

@Composable
private fun KeyboardScreen() {
    // Declare our preferred overlay height once at composition root.
    LaunchedEffect(Unit) {
        WitKeyboard.Import.requestOverlayHeight(OVERLAY_HEIGHT_PX)
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
