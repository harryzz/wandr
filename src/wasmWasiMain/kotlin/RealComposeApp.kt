// war.ime.keyboard — first-party IME app for the wart runtime.
// Renders a full keyboard (English QWERTY + Bulgarian Cyrillic +
// Symbols + Symbols2 + Emoji, all with shift / layout-cycle / etc)
// as a bottom-strip overlay surface. Pushes keystrokes to the
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

// Kept as no-op stubs so Main.kt's setup mirrors wart-app verbatim
// (both apps share Main.kt). The IME doesn't need a soft-keyboard
// bridge — IT IS the soft keyboard.
var wasiSoftKeyboardKeyHandler: (androidx.compose.ui.input.key.KeyEvent) -> Unit = {}
var wasiHideKeyboardRequest: () -> Unit = {}

fun buildRealComposeScene(widthPx: Int, heightPx: Int, density: Float): ComposeScene {
    val sceneWindowInfo = object : WindowInfo {
        override var isWindowFocused: Boolean = true
        override var keyboardModifiers: androidx.compose.ui.input.pointer.PointerKeyboardModifiers =
            androidx.compose.ui.input.pointer.PointerKeyboardModifiers(0)
        override var containerSize: IntSize = IntSize(widthPx, heightPx)
    }
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
    return scene
}

/// Task 47 step 3c — preferred panel height in physical pixels. The
/// host's wart-host receives this via `Keyboard.Import.requestOverlayHeight`,
/// forwards to `sf_resize_overlay`, and flushes the ANativeWindow's
/// buffer geometry so the next frame draws to the new size. The IME
/// is launched with an `INITIAL_OVERLAY_PX=1200` surface; this verb
/// trims it to a sensible keyboard height (~38% of a 2880-px panel).
///
/// Sized for the 5-row English / Bulgarian layouts (digits + 3
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
