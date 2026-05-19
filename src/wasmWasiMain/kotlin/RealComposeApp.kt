@file:OptIn(androidx.compose.ui.InternalComposeUiApi::class)

package testapp

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.platform.LocalInspectionMode
import androidx.compose.ui.platform.PlatformContext
import androidx.compose.ui.platform.WasiFrameDispatcher
import androidx.compose.ui.platform.WindowInfo
import androidx.compose.ui.scene.CanvasLayersComposeScene
import androidx.compose.ui.scene.ComposeScene
import androidx.compose.material3.SegmentedButton
import kotlinx.coroutines.launch

/// Single per-process dispatcher pumped by the renderer (Main.kt). Kept
/// public so the renderDelegate can call `.flush()` once per frame after
/// `scene.render(...)`. See `WasiFrameDispatcher.kt` for rationale.
val wasiFrameDispatcher: WasiFrameDispatcher = WasiFrameDispatcher()

/// Bridge from the in-canvas soft keyboard (`WasiSoftKeyboard`, drawn
/// inside the Compose composition) back to the host's `realScene`,
/// which lives in Main.kt and isn't directly reachable from composables.
/// Main.kt sets this immediately after `buildRealComposeScene()` returns.
/// Soft-keyboard taps call this; it forwards the synthetic Compose
/// `KeyEvent` into the scene (same path the hardware-keyboard handler
/// uses via `WasiInput.setKeyHandler`).
var wasiSoftKeyboardKeyHandler: (androidx.compose.ui.input.key.KeyEvent) -> Unit = {}

fun buildRealComposeScene(widthPx: Int, heightPx: Int, density: Float): ComposeScene {
    // CanvasLayersComposeScene implements ComposeSceneContext itself —
    // popup composables (DropdownMenu, AlertDialog, ExposedDropdownMenu,
    // Tooltip, …) call ComposeSceneContext.createLayer() to add a
    // z-ordered overlay onto THIS canvas (Option A in-canvas overlay
    // strategy from COMPOSE-PORTING-ANALYSIS §11.5). PlatformLayersComposeScene
    // (our previous choice) expects the platform to provide layers via
    // a real WindowManager — which we don't have on wasi.
    //
    // We must also pass a PlatformContext whose WindowInfo reports the
    // real container size in pixels. Popup's measure policy reads
    // LocalWindowInfo.current.containerSize to clip / position popups —
    // with the default PlatformContext.Empty(), containerSize stays at
    // IntSize.Zero and the popup's measure policy decides it has no
    // room → popup never lays out → not visible.
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
    val lifecycleOwner = WasiLifecycleOwnerBridge()
    val hapticFeedback = WasiHapticFeedback()
    scene.setContent {
        CompositionLocalProvider(
            LocalLifecycleOwner provides lifecycleOwner,
            androidx.compose.ui.platform.LocalHapticFeedback provides hapticFeedback,
        ) {
            MaterialDemoApp()
        }
    }
    return scene
}

@Composable
private fun MaterialDemoApp() {
    // Smoke test: prove LocalLifecycleOwner is provided (not the
    // error-throwing default) and the lifecycle-runtime-compose effect API
    // works. DisposableEffect runs once on composition + once on dispose.
    val lifecycleOwner = LocalLifecycleOwner.current
    androidx.compose.runtime.DisposableEffect(Unit) {
        org.jetbrains.skiko.wasi.wit.Canvas.Import.logMessage(
            "compose-lifecycle smoke: LocalLifecycleOwner.lifecycle.currentState=" +
                "${lifecycleOwner.lifecycle.currentState}"
        )
        onDispose { }
    }
    androidx.lifecycle.compose.LifecycleEventEffect(androidx.lifecycle.Lifecycle.Event.ON_RESUME) {
        org.jetbrains.skiko.wasi.wit.Canvas.Import.logMessage(
            "compose-lifecycle smoke: LifecycleEventEffect ON_RESUME fired"
        )
    }
    androidx.lifecycle.compose.LifecycleEventEffect(androidx.lifecycle.Lifecycle.Event.ON_PAUSE) {
        org.jetbrains.skiko.wasi.wit.Canvas.Import.logMessage(
            "compose-lifecycle smoke: LifecycleEventEffect ON_PAUSE fired"
        )
    }
    MaterialTheme(colorScheme = darkColorScheme()) {
        Surface(
            modifier = Modifier.fillMaxSize(),
            color = MaterialTheme.colorScheme.background,
        ) {
            // Bottom-overlaid soft keyboard. Scrollable content occupies
            // the upper area; the keyboard floats above it pinned to
            // bottom. Bottom padding on the scroll Column reserves space
            // so the last card isn't permanently hidden by the keyboard.
            val keyboardHeight = 300.dp
            // Hoisted so both the scroll Column (which trigers the
            // snackbar from inside SnackbarCard) AND the sibling
            // overlay Box (which renders the SnackbarHost) can see
            // the state.
            var hapticEnabled by remember { mutableStateOf(true) }
            // Manual snackbar state — bypassing SnackbarHostState +
            // SnackbarHost because that path uses
            // `LaunchedEffect(currentSnackbarData)` to auto-dismiss
            // and `mutex.withLock { suspendCancellableCoroutine }`
            // to suspend the coroutine until dismissed. On wasi the
            // LaunchedEffect on State<…> changes inside an overlay
            // doesn't reliably re-fire (same family of bugs as the
            // popup-overlay animation issue documented in
            // feedback_popup_overlay.md), so the coroutine hangs and
            // the snackbar never visibly renders. Hand-rolled
            // boolean + delay works around the whole mechanism.
            var snackbarVisible by remember { mutableStateOf(false) }
            Box(modifier = Modifier.fillMaxSize()) {
                val scrollState = rememberScrollState()
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .verticalScroll(scrollState)
                        .padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = keyboardHeight + 16.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Text(
                        text = "wasm-android-runtime",
                        fontSize = 28.sp,
                        color = MaterialTheme.colorScheme.primary,
                    )
                    Text(
                        text = "Compose Multiplatform on wasmtime + skia + EGL",
                        fontSize = 14.sp,
                        color = MaterialTheme.colorScheme.onBackground,
                    )
                    Spacer(modifier = Modifier.height(8.dp))
                    HapticScope(enabled = hapticEnabled) {
                        CounterCard()
                        CheckboxRadioCard()
                        TextFieldCard()
                        DropdownCard()
                        ProgressIndicatorsCard()
                        ListCard()
                        SliderCard()
                        FilterChipCard()
                        ChipVariantsCard()
                        FabCard()
                        // Task 28 Path D: bitmap-backed Canvas wired
                        // through to host-side skia raster surfaces +
                        // bc-* WIT verbs. SegmentedButton re-enabled
                        // 2026-05-19. DatePickerCard stays disabled —
                        // tapping its < / > chevrons consistently
                        // SIGILLs inside JIT'd wasm after a few taps,
                        // even with periodic Store::gc and a host-side
                        // bitmap-canvas LRU cap. The crash bypasses
                        // wasmtime's exception path (no Kotlin error()
                        // message is captured), so it's not in the
                        // bc-* dispatch itself — likely interacts with
                        // AnimatedContent's drawable lifecycle. Swipe
                        // to change month works fine. Tracked
                        // separately; smoke card validates all 30+
                        // bc-* verbs.
                        SegmentedButtonCard()
                        // DatePicker renders + swipe + year-pick work.
                        // Chevron `< >` taps SIGILL via Material3's
                        // IconButtonWithTooltip — a TooltipBox-on-wasi
                        // bug bisected 2026-05-19; see
                        // feedback_tooltip_sigill_wasi.md.
                        DatePickerCard()
                        Task27SmokeCard()
                        Task28SmokeCard()
                        SnackbarCard(onShow = { snackbarVisible = true })
                        SwitchRow(
                            hapticEnabled = hapticEnabled,
                            onHapticEnabledChange = { hapticEnabled = it },
                        )
                        ColorPaletteRow()
                        ButtonRow()
                    }
                }
                // Hand-rolled snackbar overlay — pinned above the
                // keyboard, auto-dismisses after 3 s via
                // LaunchedEffect.
                if (snackbarVisible) {
                    androidx.compose.runtime.LaunchedEffect(Unit) {
                        kotlinx.coroutines.delay(3000)
                        snackbarVisible = false
                    }
                    Box(
                        modifier = Modifier
                            .align(androidx.compose.ui.Alignment.BottomCenter)
                            .padding(bottom = keyboardHeight + 8.dp, start = 16.dp, end = 16.dp),
                    ) {
                        androidx.compose.material3.Snackbar(
                            action = {
                                // Snackbar background is
                                // `colorScheme.inverseSurface`; the
                                // action button needs an explicit
                                // inversePrimary content color to be
                                // legible on it. Default text-button
                                // colors use `colorScheme.primary`
                                // which collides with the dark snackbar.
                                HapticButton(
                                    onClick = { snackbarVisible = false },
                                    colors = ButtonDefaults.textButtonColors(
                                        contentColor = MaterialTheme.colorScheme.inversePrimary,
                                    ),
                                ) { Text("Undo") }
                            },
                        ) {
                            Text("Snackbar from a HapticButton tap")
                        }
                    }
                }
                // Soft keyboard overlay — drawn last so it sits on top.
                Box(modifier = Modifier.align(androidx.compose.ui.Alignment.BottomCenter)) {
                    WasiSoftKeyboard(
                        onKey = { ev -> wasiSoftKeyboardKeyHandler(ev) },
                        height = keyboardHeight,
                    )
                }
            }
        }
    }
}

@Composable
private fun CounterCard() {
    var count by remember { mutableStateOf(0) }
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
        ),
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text(
                text = "Counter",
                fontSize = 18.sp,
                color = MaterialTheme.colorScheme.onSurface,
            )
            Spacer(modifier = Modifier.height(8.dp))
            Row(
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                HapticButton(onClick = { count-- }) { Text("-") }
                Text(
                    text = "$count",
                    fontSize = 24.sp,
                    color = MaterialTheme.colorScheme.onSurface,
                )
                HapticButton(onClick = { count++ }) { Text("+") }
            }
        }
    }
}

@Composable
private fun SliderCard() {
    var v by remember { mutableStateOf(0.4f) }
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text("Slider value: ${(v * 100).toInt()}%", fontSize = 14.sp)
            // steps = 9 → 11 discrete positions (0 %, 10 %, … 100 %).
            // HapticSlider fires haptic on each step crossing during
            // drag, gated through HapticScope. Default SegmentTick
            // maps to TICK+LIGHT — perceptible only on a still hand;
            // bump to Confirm (CLICK+MEDIUM) so it's actually felt
            // while dragging on a Pixel 2 XL.
            HapticSlider(
                value = v,
                onValueChange = { v = it },
                steps = 9,
                feedback = androidx.compose.ui.hapticfeedback.HapticFeedbackType.Confirm,
            )
        }
    }
}

@Composable
private fun FilterChipCard() {
    val tags = listOf("Coffee", "Tea", "Juice", "Water")
    var selected by remember { mutableStateOf(setOf("Coffee", "Tea")) }
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text("Filter chips", fontSize = 16.sp)
            Spacer(modifier = Modifier.height(8.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                tags.forEach { tag ->
                    HapticFilterChip(
                        selected = tag in selected,
                        onClick = {
                            selected =
                                if (tag in selected) selected - tag else selected + tag
                        },
                        label = { Text(tag) },
                    )
                }
            }
        }
    }
}

// Material3 DatePicker. Uses the popup-overlay path internally for
// year/month navigation, so the existing `LocalInspectionMode
// provides true` workaround from DropdownCard may be needed if the
// year-selection animation doesn't drive recompose. Inline (not in
// a Dialog) so it shows up directly in the scroll column.
@Composable
private fun DatePickerCard() {
    val state = androidx.compose.material3.rememberDatePickerState()
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text("Date picker", fontSize = 16.sp)
            Spacer(modifier = Modifier.height(8.dp))
            androidx.compose.runtime.CompositionLocalProvider(
                LocalInspectionMode provides true,
            ) {
                androidx.compose.material3.DatePicker(
                    state = state,
                    showModeToggle = false,
                    title = null,
                    headline = null,
                )
            }
        }
    }
}

@Composable
private fun SnackbarCard(onShow: () -> Unit) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text("Snackbar", fontSize = 16.sp)
            Spacer(modifier = Modifier.height(8.dp))
            HapticButton(onClick = onShow) { Text("Show snackbar") }
        }
    }
}

@Composable
private fun SegmentedButtonCard() {
    val options = listOf("Day", "Week", "Month")
    var selected by remember { mutableStateOf(1) }
    val haptic = androidx.compose.ui.platform.LocalHapticFeedback.current
    val hapticEnabled = LocalHapticEnabled.current
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text("Segmented button", fontSize = 16.sp)
            Spacer(modifier = Modifier.height(8.dp))
            // SegmentedButton is an extension on
            // SingleChoiceSegmentedButtonRowScope; calling the
            // imported function inside the row's content lambda
            // resolves the receiver automatically.
            androidx.compose.material3.SingleChoiceSegmentedButtonRow {
                options.forEachIndexed { index, label ->
                    SegmentedButton(
                        selected = index == selected,
                        onClick = {
                            if (hapticEnabled) {
                                haptic.performHapticFeedback(
                                    androidx.compose.ui.hapticfeedback.HapticFeedbackType.Confirm
                                )
                            }
                            selected = index
                        },
                        shape = androidx.compose.material3.SegmentedButtonDefaults.itemShape(
                            index = index,
                            count = options.size,
                        ),
                    ) { Text(label) }
                }
            }
        }
    }
}

@Composable
private fun ChipVariantsCard() {
    // Three more chip types alongside the existing FilterChip:
    //   * AssistChip      → non-toggling, suggests a one-shot action
    //   * SuggestionChip  → non-toggling, content-recommendation hint
    //   * InputChip       → toggling, used for tag-style input (we use
    //                       it as a selectable "tag" here)
    var inputSelected by remember { mutableStateOf(true) }
    var assistTaps by remember { mutableStateOf(0) }
    var suggestionTaps by remember { mutableStateOf(0) }
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text("Chip variants", fontSize = 16.sp)
            Spacer(modifier = Modifier.height(8.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                HapticAssistChip(
                    onClick = { assistTaps++ },
                    label = { Text("Assist ($assistTaps)") },
                )
                HapticSuggestionChip(
                    onClick = { suggestionTaps++ },
                    label = { Text("Sugg ($suggestionTaps)") },
                )
                HapticInputChip(
                    selected = inputSelected,
                    onClick = { inputSelected = !inputSelected },
                    label = { Text("Input") },
                )
            }
        }
    }
}

@Composable
private fun FabCard() {
    var fabTaps by remember { mutableStateOf(0) }
    var extTaps by remember { mutableStateOf(0) }
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text("Floating action buttons", fontSize = 16.sp)
            Spacer(modifier = Modifier.height(8.dp))
            Row(
                horizontalArrangement = Arrangement.spacedBy(16.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                HapticFloatingActionButton(onClick = { fabTaps++ }) {
                    Text("+", fontSize = 24.sp)
                }
                Text("FAB: $fabTaps", fontSize = 14.sp)
                HapticExtendedFloatingActionButton(
                    onClick = { extTaps++ },
                    text = { Text("Extended ($extTaps)") },
                )
            }
        }
    }
}

@Composable
private fun SwitchRow(
    hapticEnabled: Boolean,
    onHapticEnabledChange: (Boolean) -> Unit,
) {
    Card(modifier = Modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .clickable { onHapticEnabledChange(!hapticEnabled) }
                .padding(horizontal = 16.dp, vertical = 12.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("Enable haptic", fontSize = 16.sp)
            Switch(checked = hapticEnabled, onCheckedChange = onHapticEnabledChange)
        }
    }
}

@Composable
private fun ColorPaletteRow() {
    val palette = listOf(
        Color(0xFFEF4444), Color(0xFFF59E0B), Color(0xFF10B981),
        Color(0xFF3B82F6), Color(0xFF8B5CF6), Color(0xFFEC4899),
    )
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text("Palette", fontSize = 16.sp)
            Spacer(modifier = Modifier.height(8.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                palette.forEach { c ->
                    Box(
                        modifier = Modifier
                            .size(36.dp)
                            .background(c, RoundedCornerShape(8.dp)),
                    )
                }
            }
        }
    }
}

@Composable
private fun ButtonRow() {
    // Uses `HapticButton` (HapticWidgets.kt) which reads
    // `LocalHapticEnabled` from the surrounding provider. Primary
    // gets the default Confirm feedback (CLICK + MEDIUM); Secondary
    // upgrades to LongPress (HEAVY_CLICK + STRONG) — same mapping
    // as the B5 device-verify, just routed through the shared
    // design-system layer instead of manual.
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        HapticButton(onClick = {}) { Text("Primary") }
        HapticButton(
            onClick = {},
            feedback = androidx.compose.ui.hapticfeedback.HapticFeedbackType.LongPress,
            colors = ButtonDefaults.outlinedButtonColors(),
        ) { Text("Secondary") }
    }
}

// BasicTextField recon (task #52, 2026-05-13):
//   * Renders cleanly — no composition crash.
//   * Cursor blink fix: `WasiFrameDispatcher` now implements `Delay`, so
//     `CursorAnimationState`'s `while(true) { delay(500) }` blink loop
//     properly suspends instead of busy-looping.
//   * REMAINING FREEZE: tap-to-focus on a real BasicTextField still hangs
//     the wasm main thread at 100% CPU. Bisect: focusable() alone — no
//     freeze; clickable {} alone — no freeze; BasicTextField with all
//     variants tried (readOnly, transparent cursor) — freezes. So the
//     spin is inside the synchronous onTap path (focusRequester.requestFocus
//     → focus state change → recompose → ???) before any visible coroutine
//     dispatch fires. Filed in `feedback_basictextfield_freeze.md`.
//
// Try TextFieldState-based BasicTextField (formerly BasicTextField2).
// Different state plumbing than legacy `BasicTextField(value, onValueChange)`
// — may dodge the legacy `defaultTextFieldPointer` synchronous-onTap
// freeze.
@Composable
private fun TextFieldCard() {
    val state = androidx.compose.foundation.text.input.rememberTextFieldState("hello world")
    // EXPERIMENT: read state.selection at the COMPOSABLE level so that any
    // selection change forces a recomposition of TextFieldCard, which
    // re-emits the entire BasicTextField subtree. If the cursor visually
    // moves after this read, it confirms the bug is "BasicTextField's
    // internal draw-phase observer doesn't see selection changes" and
    // forcing recomposition is a viable workaround.
    val sel = state.selection
    // DIAG for cursor-position render bug: log every observed selection change.
    androidx.compose.runtime.LaunchedEffect(state) {
        kotlinx.coroutines.flow.combine(
            androidx.compose.runtime.snapshotFlow { state.text.toString() },
            androidx.compose.runtime.snapshotFlow { state.selection },
        ) { t, s -> "text=\"$t\" sel=$s" }.collect {
            org.jetbrains.skiko.wasi.wit.Canvas.Import.logMessage("tfstate $it")
        }
    }
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text("BasicTextField (TextFieldState API)", fontSize = 16.sp)
            Spacer(modifier = Modifier.height(8.dp))
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .background(
                        color = MaterialTheme.colorScheme.surfaceVariant,
                        shape = RoundedCornerShape(4.dp),
                    )
                    .padding(horizontal = 16.dp, vertical = 14.dp),
            ) {
                androidx.compose.foundation.text.BasicTextField(
                    state = state,
                    modifier = Modifier.fillMaxWidth(),
                    textStyle = androidx.compose.ui.text.TextStyle(
                        fontSize = 16.sp,
                        color = MaterialTheme.colorScheme.onSurface,
                    ),
                    // Default cursor brush is SolidColor(Color.Black) which
                    // is invisible against the dark Material theme background.
                    // Use primary so it pops on both light & dark surfaces.
                    cursorBrush = androidx.compose.ui.graphics.SolidColor(MaterialTheme.colorScheme.primary),
                )
            }
        }
    }
}

// Two small state widgets in one card.
@Composable
private fun CheckboxRadioCard() {
    var selected by remember { mutableStateOf(0) }
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            // Extract the checkbox row into its own composable. WORKAROUND
            // for a wasi-specific recompose-scope bug: when state + state-
            // read + state-mutation were all inline at this top-level
            // function body, only the FIRST recompose fired after a state
            // change — subsequent toggles never invalidated this scope, so
            // the visual stuck at the first toggled value. The `forEach`
            // for radio rows below works because its lambda is already its
            // own restart group. Extracting into a function gives the
            // checkbox row its own restart group too. Tracked as
            // compose-runtime-wasi bug.
            CheckboxRow()
            HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp))
            Text("Pick a flavor", fontSize = 14.sp)
            listOf("Vanilla", "Chocolate", "Strawberry").forEachIndexed { i, name ->
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .hapticSelectable(
                            selected = selected == i,
                            role = androidx.compose.ui.semantics.Role.RadioButton,
                        ) { selected = i }
                        .padding(vertical = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    RadioButton(selected = selected == i, onClick = null)
                    Spacer(modifier = Modifier.size(8.dp))
                    Text(name, fontSize = 14.sp)
                }
            }
        }
    }
}

// Checkbox row — owns its own state (`checked`) so it has an
// independent restart group. See CheckboxRadioCard for the rationale
// (compose-runtime-wasi recompose-scope bug). Uses plain
// Modifier.clickable + non-interactive Checkbox (onCheckedChange =
// null) to avoid double-fire from two click handlers.
@Composable
private fun CheckboxRow() {
    // DIAGNOSTIC: same exact pattern as Counter (`var x by remember { mutableStateOf(0) }`
    // toggled via Button.onClick). Two tests side by side:
    //   - `inc`: 0,1,2,3,4… (Counter pattern)
    //   - `flip`: 0,1,0,1,0 (toggle pattern, also Int)
    // If `inc` updates per tap but `flip` does NOT, the recompose bug is
    // specifically about state writes that return to a prior value
    // (Compose's snapshot dedup or equality short-circuit).
    var checked by remember { mutableStateOf(true) }
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .hapticClickable { checked = !checked }
            .padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Checkbox(checked = checked, onCheckedChange = null)
        Spacer(modifier = Modifier.size(8.dp))
        Text("I agree to the terms", fontSize = 14.sp)
    }
}

// Anchored menu — exercises Compose Popup overlay end-to-end.
// `LocalInspectionMode provides true` forces Material3's Menu to use
// `expandedState.targetState`-conditional snap instead of the
// animated alpha/scale path. Required because on wasi the
// `Modifier.graphicsLayer { State<Float> }` block is invoked once at
// initial composition and never re-invoked when the State updates —
// only inside layer (popup/dialog) compositions. Main composition
// works. See `feedback_popup_overlay.md` for the precise repro.
@Composable
private fun DropdownCard() {
    var expanded by remember { mutableStateOf(false) }
    var choice by remember { mutableStateOf("None") }
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text("Dropdown menu", fontSize = 16.sp)
            Spacer(modifier = Modifier.height(8.dp))
            Box {
                HapticButton(onClick = { expanded = true }) {
                    Text("Selected: $choice")
                }
                // Task #53 PARTIALLY fixed 2026-05-13:
                //   * Pure `Popup` + `Modifier.graphicsLayer { State }`
                //     animation NOW WORKS — block re-invokes per frame
                //     with the live State<Float> value. Confirmed via
                //     test-app diagnostic logs (gfx-block #1..1110 with
                //     a progressing 0→1).
                //   * Material3 `DropdownMenu` STILL needs the
                //     `LocalInspectionMode provides true` workaround. It
                //     uses `MutableTransitionState` / `updateTransition`
                //     for its expand-collapse animation — a different
                //     animation path than `animateFloatAsState`, and
                //     something in that path still doesn't update layer
                //     visibility in popup compositions.
                CompositionLocalProvider(LocalInspectionMode provides true) {
                    DropdownMenu(
                        expanded = expanded,
                        onDismissRequest = { expanded = false },
                    ) {
                        listOf("Alpha", "Beta", "Gamma").forEach { name ->
                            HapticDropdownMenuItem(
                                text = { Text(name) },
                                onClick = { choice = name; expanded = false },
                            )
                        }
                    }
                }
            }
        }
    }
}

// LEAK BISECT 2026-05-13:
//   * indeterminate Material3 progress: ~2 MB/s leak, 28% CPU
//   * static  Material3 progress      : ~0.12 MB/s, 14% CPU
//   * THIS CARD: replace Material3 indicators with a custom Canvas +
//     rememberInfiniteTransition + drawLine. If THIS leaks too, the
//     issue is in the wasi WIT draw path itself, not in Material3.
//     If it doesn't, Material3's specific wrapping (semantics, etc.)
//     is the culprit.
// Static progress indicators. Indeterminate variants leak ~2.5 MB/s
// in wasm linear memory (Kotlin/Wasm codegen retains continuation refs;
// confirmed unchanged between Kotlin 2.4.0-Beta2 and 2.4.0-RC).
// See `feedback_indeterminate_progress_leak.md`.
@Composable
private fun ProgressIndicatorsCard() {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text("Progress", fontSize = 16.sp)
            Spacer(modifier = Modifier.height(12.dp))
            LinearProgressIndicator(progress = { 0.5f }, modifier = Modifier.fillMaxWidth())
            Spacer(modifier = Modifier.height(16.dp))
            Row(
                horizontalArrangement = Arrangement.spacedBy(16.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                CircularProgressIndicator(progress = { 0.5f })
                Text("50%", fontSize = 14.sp)
            }
        }
    }
}

// LazyColumn — virtualized item list. Verified 2026-05-13 on device:
// renders + nested-scrolls correctly with the host-side transforms
// architecture (task #51).
@Composable
private fun ListCard() {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text("List (Lazy, 30 items)", fontSize = 16.sp)
            Spacer(modifier = Modifier.height(8.dp))
            androidx.compose.foundation.lazy.LazyColumn(
                modifier = Modifier.fillMaxWidth().height(280.dp),
            ) {
                items(30) { n ->
                    Text(
                        text = "Item #${n + 1}",
                        fontSize = 14.sp,
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(vertical = 6.dp),
                    )
                    if (n < 29) HorizontalDivider()
                }
            }
        }
    }
}
