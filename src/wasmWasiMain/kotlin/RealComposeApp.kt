// war.ime.keyboard — task 47 step 3b. First-party IME app for the
// wart runtime. Renders a minimal keyboard surface (a row of letter
// buttons + space + backspace + enter) and pushes keystrokes to the
// currently-focused editor in another wart guest via the arbiter
// (Keyboard.Import.sendKeyEvent → arbiter ime-send-key-event →
// focused-pid's per-host control socket → dispatch_key_v2).
//
// MVP scope: single-letter buttons. Architecture proves the
// IME→arbiter→focused-editor delivery loop. Full QWERTY + shift +
// numbers + punctuation lands as iterations once the path is
// proven. See `tasks/47-ime-via-guest-app.md` step 3b.

@file:OptIn(androidx.compose.ui.InternalComposeUiApi::class)

package testapp

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.PlatformContext
import androidx.compose.ui.platform.WasiFrameDispatcher
import androidx.compose.ui.platform.WindowInfo
import androidx.compose.ui.scene.CanvasLayersComposeScene
import androidx.compose.ui.scene.ComposeScene
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import org.jetbrains.skiko.wasi.wit.Canvas as WitCanvas
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
private const val OVERLAY_HEIGHT_PX: UInt = 1100u

@Composable
private fun KeyboardScreen() {
    // Task 47 step 3c — declare our preferred overlay height once at
    // composition root. The host re-sizes the SurfaceControl to a
    // bottom strip; the rest of the screen shows the focused editor
    // (Compose layout continues to fill whatever surface we've been
    // given via WindowInfo.containerSize). Fire-and-forget — failures
    // log host-side but don't break the keyboard UI.
    LaunchedEffect(Unit) {
        WitKeyboard.Import.requestOverlayHeight(OVERLAY_HEIGHT_PX)
    }

    // Anchor the keyboard to the bottom of the surface, where real
    // IMEs sit. With the overlay surface trimmed (step 3c above), the
    // outer Box IS the keyboard panel — the `BottomCenter` anchor is
    // redundant but harmless. The dark background fills the panel.
    Box(
        modifier = Modifier.fillMaxSize().background(Color(0xFF1F1F1F)),
        contentAlignment = Alignment.BottomCenter,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = "war.ime.keyboard — first-party IME (task 47 step 3b)",
                color = Color.White,
                fontSize = 14.sp,
                modifier = Modifier.padding(bottom = 8.dp),
            )

            // Top row: a b c d e
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                ImeKey("a", codePoint = 97, keyId = 29, modifier = Modifier.weight(1f))
                ImeKey("b", codePoint = 98, keyId = 30, modifier = Modifier.weight(1f))
                ImeKey("c", codePoint = 99, keyId = 31, modifier = Modifier.weight(1f))
                ImeKey("d", codePoint = 100, keyId = 32, modifier = Modifier.weight(1f))
                ImeKey("e", codePoint = 101, keyId = 33, modifier = Modifier.weight(1f))
            }

            // Useful for typing "hello world".
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                ImeKey("h", codePoint = 104, keyId = 36, modifier = Modifier.weight(1f))
                ImeKey("l", codePoint = 108, keyId = 40, modifier = Modifier.weight(1f))
                ImeKey("o", codePoint = 111, keyId = 43, modifier = Modifier.weight(1f))
                ImeKey("w", codePoint = 119, keyId = 51, modifier = Modifier.weight(1f))
                ImeKey("r", codePoint = 114, keyId = 46, modifier = Modifier.weight(1f))
            }

            // Bottom row: space + backspace + enter.
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                ImeKey("space", codePoint = 32, keyId = 62, modifier = Modifier.weight(2f))
                ImeKey("⌫", codePoint = 0, keyId = 67, modifier = Modifier.weight(1f))
                ImeKey("⏎", codePoint = 0, keyId = 66, modifier = Modifier.weight(1f))
            }
        }
    }
}

@Composable
private fun ImeKey(
    label: String,
    codePoint: Int,
    keyId: Int,
    modifier: Modifier = Modifier,
) {
    // We dispatch down + up on tap — the focused editor sees a full
    // press cycle. Compose treats this consistently for character
    // input. Step 3c's full QWERTY can split press / hold-to-repeat
    // / etc.
    Box(
        modifier = modifier
            .height(72.dp)
            .background(Color(0xFF2F2F2F))
            .pointerInput(codePoint, keyId) {
                detectTapGestures(onTap = {
                    WitCanvas.Import.logMessage(
                        "ime-key tap: label=$label codePoint=$codePoint keyId=$keyId"
                    )
                    WitKeyboard.Import.sendKeyEvent(
                        codePoint.toUInt(), keyId.toUInt(), /*action down=*/ 0u,
                    )
                    WitKeyboard.Import.sendKeyEvent(
                        codePoint.toUInt(), keyId.toUInt(), /*action up=*/ 1u,
                    )
                })
            },
        contentAlignment = Alignment.Center,
    ) {
        Text(
            text = label,
            color = Color.White,
            fontSize = 18.sp,
            fontWeight = FontWeight.Medium,
            textAlign = TextAlign.Center,
        )
    }
}
