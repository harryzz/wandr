@file:OptIn(
    kotlin.wasm.unsafe.UnsafeWasmMemoryApi::class,
    kotlin.wasm.unsafe.ComponentModelInternalApi::class,
    androidx.compose.ui.InternalComposeUiApi::class,
    androidx.compose.ui.ExperimentalComposeUiApi::class,
)

package testapp

import kotlin.wasm.unsafe.*

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.asComposeCanvas
import org.jetbrains.skia.Image
import org.jetbrains.skiko.SkiaLayer
import org.jetbrains.skiko.SkikoInputDelegate
import org.jetbrains.skiko.SkikoKeyEventKind
import org.jetbrains.skiko.SkikoPointerEventKind
import org.jetbrains.skiko.SkikoRenderDelegate
import org.jetbrains.skiko.currentSkiaLayer
import org.jetbrains.skiko.wasi.WasiInput
import org.jetbrains.skiko.wasi.WasiLifecycle
import org.jetbrains.skiko.wasi.WasiScheduler
import org.jetbrains.skiko.wasi.wit.Canvas as WitCanvas
import org.jetbrains.skiko.wasi.wit.Clipboard as WitClipboard
import org.jetbrains.skiko.wasi.wit.Haptics as WitHaptics
import org.jetbrains.skiko.wasi.wit.Power as WitPower
import org.jetbrains.skiko.wasi.wit.Sensors as WitSensors
import org.jetbrains.skiko.wasi.wit.Thermal as WitThermal
import org.jetbrains.skiko.wasi.wit.Locale as WitLocale
import org.jetbrains.skiko.wasi.wit.PointerIcon as WitPointerIcon
import org.jetbrains.skiko.wasi.wit.TextSegmentation as WitTextSeg
import org.jetbrains.skiko.wasi.wit.Window as WitWindow
import testapp.compose.App
import testapp.compose.WasiComposeScene

fun main() {
    composeUiBaseSmokeTest()
    composeUiGraphicsSmokeTest()
    composeUiTextSmokeTest()
    composeUiSmokeTest()
    composeFoundationLayoutSmokeTest()
    composeAnimationCoreSmokeTest()
    composeFoundationSmokeTest()
    composeMaterialRippleSmokeTest()
    composeMaterial3SmokeTest()

    val density = WitWindow.Import.getDensity()
    val fontScale = WitWindow.Import.getFontScale()
    val dpi = WitWindow.Import.getDpi()
    WitCanvas.Import.logMessage(
        "android-window smoke: density=${density} px/dp, fontScale=${fontScale}, dpi=${dpi}"
    )

    val h1 = WasiScheduler.schedule(800u) {
        WitCanvas.Import.logMessage("android-scheduler smoke: fired @800ms ✓")
    }
    val h2 = WasiScheduler.schedule(200u) {
        WitCanvas.Import.logMessage("android-scheduler smoke: BUG — cancelled task still fired")
    }
    WasiScheduler.cancel(h2)
    WitCanvas.Import.logMessage(
        "android-scheduler smoke: h1=${h1} scheduled @800ms, h2=${h2} cancelled @200ms"
    )

    // text-segmentation smoke: exercise grapheme/word/sentence on a string
    // with multi-byte UTF-8 (¿Cómo estás? has multi-byte chars).
    val sample = "Hello, world! ¿Cómo estás?"
    val sampleLen = sample.encodeToByteArray().size.toUInt()
    val wordBoundaries = mutableListOf<UInt>()
    var cur = 0u
    while (cur < sampleLen) {
        val next = WitTextSeg.Import.nextBoundary(sample, WitTextSeg.BoundaryKind.WORD, cur + 1u)
        if (next == cur || next > sampleLen) break
        wordBoundaries.add(next)
        if (next == sampleLen) break
        cur = next
    }
    val sentenceFirst = WitTextSeg.Import.nextBoundary(sample, WitTextSeg.BoundaryKind.SENTENCE, 1u)
    val sentenceSecond = WitTextSeg.Import.nextBoundary(sample, WitTextSeg.BoundaryKind.SENTENCE, sentenceFirst + 1u)
    val graphemeAt3 = WitTextSeg.Import.nextBoundary(sample, WitTextSeg.BoundaryKind.GRAPHEME, 3u)
    val prevWordBefore16 = WitTextSeg.Import.prevBoundary(sample, WitTextSeg.BoundaryKind.WORD, 16u)
    WitCanvas.Import.logMessage(
        "text-segmentation smoke: word-bounds=${wordBoundaries}, " +
        "sentences-end-at=[${sentenceFirst}, ${sentenceSecond}], " +
        "grapheme-≥3=${graphemeAt3}, prev-word-≤16=${prevWordBefore16}, " +
        "total-utf8-bytes=${sampleLen}"
    )

    // android-input-v2 smoke: log first 5 enriched events that arrive.
    var v2Count = 0
    WasiInput.setPointerHandler { pointerId, kind, x, y, pressure ->
        if (v2Count < 5) {
            v2Count++
            WitCanvas.Import.logMessage(
                "android-input-v2 smoke #${v2Count}: id=${pointerId} kind=${kind} " +
                "@(${x.toInt()},${y.toInt()}) pressure=${pressure}"
            )
        }
    }

    // android-lifecycle smoke: log current state + register an observer.
    WitCanvas.Import.logMessage(
        "android-lifecycle smoke: currentState=${WasiLifecycle.currentState()} at boot"
    )
    WasiLifecycle.addObserver { state ->
        WitCanvas.Import.logMessage("android-lifecycle smoke: → ${state}")
    }

    // android-power + android-thermal smoke: probe IPower + IThermal HALs.
    // Backed by android.hardware.{power.IPower,thermal.IThermal}/default
    // via rsbinder (task 19). Expected true on rooted Pixel with setenforce
    // 0; on stock devices SELinux untrusted_app→hal_{power,thermal}_default
    // is denied so set/boost are no-ops and queries return false / empty.
    val intHintSup = WitPower.Import.isHintSupported(WitPower.Hint.INTERACTION)
    WitPower.Import.boost(WitPower.Hint.INTERACTION, 100u)
    val overall    = WitThermal.Import.overallThrottle()
    val allTemps   = WitThermal.Import.listTemperatures()
    WitCanvas.Import.logMessage(
        "android-power-thermal smoke: hint(INTERACTION) supported=${intHintSup}, boost sent, " +
        "overallThrottle=${overall}, sensors=${allTemps.size}" +
        (if (allTemps.isNotEmpty()) " first=${allTemps.first().kind}/${allTemps.first().celsius}°C" else "")
    )

    // android-sensors smoke: enumerate sensors + enable accel briefly +
    // poll one reading. Backed by android.frameworks.sensorservice.
    // ISensorManager/default via rsbinder (task 20). The pollLatest()
    // fires from a delayed coroutine because the HAL needs time after
    // enableSensor() to deliver the first event.
    val sensors = WitSensors.Import.listSensors()
    val accelInfo = sensors.firstOrNull { it.kind == WitSensors.Kind.ACCELEROMETER }
    WitCanvas.Import.logMessage(
        "android-sensors smoke: ${sensors.size} sensors; accel handle=${accelInfo?.handle ?: 0u}"
    )
    if (accelInfo != null) {
        WitSensors.Import.enable(accelInfo.handle, 50u)
        WasiScheduler.schedule(200u) {
            val s = WitSensors.Import.pollLatest(accelInfo.handle)
            WitCanvas.Import.logMessage(
                "android-sensors smoke: accel ts=${s.timestampNs} x=${s.x} y=${s.y} z=${s.z} (m/s²)"
            )
            WitSensors.Import.disable(accelInfo.handle)
        }
    }

    // android-haptics smoke: try a tap + an explicit 50ms vibrate.
    // Backed by android.hardware.vibrator.IVibrator/default via rsbinder
    // (task 16). Expected true on rooted Pixel with `setenforce 0`; on
    // stock devices the SELinux untrusted_app→hal_vibrator_default policy
    // denies the binder call → returns false (no crash).
    val tapOk      = WitHaptics.Import.perform(WitHaptics.Feedback.TAP)
    val vibrateOk  = WitHaptics.Import.vibrateMs(50u)
    WitCanvas.Import.logMessage(
        "android-haptics smoke: perform(TAP)=${tapOk}, vibrateMs(50)=${vibrateOk}"
    )

    // (Removed the startup android-audio smoke beep: it leaked an AAudio MMAP
    // track it never closed — keeping audioserver pumping at ~8-9% CPU for the
    // life of the app. The audio path is now exercised by the user-triggered
    // "Play Tone" button in CounterDemo, which plays AND closes the track so
    // audioserver returns to idle.)

    // android-locale smoke: read user's locale, time format, direction.
    val loc       = WitLocale.Import.primaryLocale()
    val is24      = WitLocale.Import.is24HourFormat()
    val direction = WitLocale.Import.getLayoutDirection()
    WitCanvas.Import.logMessage(
        "android-locale smoke: primary=${loc}, 24h=${is24}, direction=${direction}"
    )

    // android-clipboard smoke: starts empty, write, read back, clear, read.
    val before = WitClipboard.Import.hasText()
    WitClipboard.Import.setText("Hello, clipboard! 复制粘贴 ⌘C")
    val mid    = WitClipboard.Import.getText()
    val hasMid = WitClipboard.Import.hasText()
    WitClipboard.Import.clear()
    val after  = WitClipboard.Import.hasText()
    WitCanvas.Import.logMessage(
        "android-clipboard smoke: before=hasText=${before}, " +
        "after-set: text=\"${mid}\" hasText=${hasMid}, " +
        "after-clear: hasText=${after}"
    )

    // android-pointer-icon smoke: must not crash on touch-only device.
    WitPointerIcon.Import.set(WitPointerIcon.Kind.TEXT)
    WitPointerIcon.Import.set(WitPointerIcon.Kind.HAND)
    WitPointerIcon.Import.set(WitPointerIcon.Kind.DEFAULT)
    WitCanvas.Import.logMessage("android-pointer-icon smoke: 3 calls completed (no-op on touch)")

    val layer = SkiaLayer()
    currentSkiaLayer = layer

    // Reusable images created once. Compose composables read these as inputs.
    val checkerW = 32; val checkerH = 32
    val checkerPixels = ByteArray(checkerW * checkerH * 4)
    for (py in 0 until checkerH) {
        for (px in 0 until checkerW) {
            val i = (py * checkerW + px) * 4
            val light = ((px / 8 + py / 8) % 2 == 0)
            checkerPixels[i + 0] = if (light) 0xFF.toByte() else 0x44.toByte()
            checkerPixels[i + 1] = if (light) 0xFF.toByte() else 0x88.toByte()
            checkerPixels[i + 2] = if (light) 0xFF.toByte() else 0xFF.toByte()
            checkerPixels[i + 3] = 0xFF.toByte()
        }
    }
    val checkerImg = Image.makeFromPixels(checkerW, checkerH, checkerPixels)
    val whiteImg = Image.makeFromPixels(32, 32, ByteArray(32 * 32 * 4) { 0xFF.toByte() })

    // Top-level state read by Showcase. Updated from inputDelegate; the
    // dispatch* methods on WasiComposeScene call Snapshot.sendApplyNotifications,
    // which propagates these writes to the recomposer.
    var pointerXState   by mutableStateOf(0f)
    var pointerYState   by mutableStateOf(0f)
    var pointerDownState by mutableStateOf(false)
    var lastKeyState    by mutableStateOf(-1)

    // ── Real Compose Multiplatform scene (Material3 demo) ────────────────
    val widthPx  = WitCanvas.Import.surfaceWidth().toInt()
    val heightPx = WitCanvas.Import.surfaceHeight().toInt()
    val realScene = buildRealComposeScene(widthPx, heightPx, density)
    WitCanvas.Import.logMessage(
        "real-compose scene built: ${widthPx}x${heightPx} px @ density=${density}"
    )

    // Bridge from the in-canvas soft keyboard back to the scene. flush()
    // is needed so state mutations triggered by the KeyEvent propagate
    // before the next pointer event arrives — without it the gesture
    // coroutine's next withTimeoutOrNull{ waitForUpOrCancellation } stalls
    // on a delay that won't fire until something else triggers a flush.
    wasiSoftKeyboardKeyHandler = { keyEvent ->
        realScene.sendKeyEvent(keyEvent)
        wasiFrameDispatcher.flush()
    }

    layer.inputDelegate = object : SkikoInputDelegate {
        override fun onPointerEvent(kind: SkikoPointerEventKind, x: Float, y: Float) {
            val type = androidx.compose.ui.input.pointer.PointerEventType
            val evtType = when (kind) {
                SkikoPointerEventKind.DOWN   -> type.Press
                SkikoPointerEventKind.UP     -> type.Release
                SkikoPointerEventKind.MOVE   -> type.Move
                SkikoPointerEventKind.SCROLL -> type.Scroll
                else                         -> type.Move
            }
            realScene.sendPointerEvent(
                eventType = evtType,
                position = androidx.compose.ui.geometry.Offset(x, y),
                type = androidx.compose.ui.input.pointer.PointerType.Touch,
            )
            // Drain the dispatcher after EVERY pointer event so the
            // suspending pointer-input coroutines (Modifier.scrollable,
            // detectDragGestures, …) get to await the NEXT event before
            // it arrives. Without this, only DOWN/UP make it to the
            // scrollable's drag detector — MOVE deltas pile up as
            // queued resumptions that all share the same stale
            // continuation, and scrolling never engages.
            wasiFrameDispatcher.flush()
        }
        override fun onKeyEvent(kind: SkikoKeyEventKind, keyCode: Int) {
            // Legacy v1 path — keep but unused now that on-key-event-v2
            // delivers the resolved codepoint + Compose-compatible key-id.
        }
    }
    // Wire enriched hardware-keyboard events to the Compose scene. The
    // host emits BOTH v1 (onKeyEvent above) and v2 (onKeyEventV2 here);
    // v2 carries the resolved UTF-32 codePoint plus a numeric key-id
    // whose values match upstream Compose webMain `Key(...)` constants,
    // so we can build a Compose KeyEvent directly without a translation
    // table. After this, `adb shell input keyevent KEYCODE_A` types "a"
    // into the focused BasicTextField (the TextFieldState API one — the
    // legacy onValueChange API still freezes on tap, see
    // feedback_basictextfield_freeze.md).
    org.jetbrains.skiko.wasi.WasiInput.setKeyHandler { kind, codePoint, keyId ->
        @OptIn(androidx.compose.ui.InternalComposeUiApi::class)
        val type = if (kind == org.jetbrains.skiko.wasi.wit.Renderer.KeyKind.DOWN) {
            androidx.compose.ui.input.key.KeyEventType.KeyDown
        } else {
            androidx.compose.ui.input.key.KeyEventType.KeyUp
        }
        // ESC dismisses the in-canvas soft keyboard. We do this on KeyDown
        // before forwarding so the dismiss happens once per press, and we
        // still forward so any field-level handler / focus owner that
        // wants to see the ESC also gets it.
        if (keyId == 27u && type == androidx.compose.ui.input.key.KeyEventType.KeyDown) {
            testapp.wasiHideKeyboardRequest()
        }
        // Use the printable codePoint when present; otherwise the named-key id.
        val key = androidx.compose.ui.input.key.Key(
            if (keyId != 0u) keyId.toLong() else codePoint.toLong()
        )
        @OptIn(androidx.compose.ui.InternalComposeUiApi::class)
        val keyEvent = androidx.compose.ui.input.key.KeyEvent(
            key = key,
            type = type,
            codePoint = codePoint.toInt(),
        )
        realScene.sendKeyEvent(keyEvent)
        wasiFrameDispatcher.flush()
    }

    layer.renderDelegate = SkikoRenderDelegate { canvas, w, h, nanos ->
        androidx.compose.ui.updateCachedNanoTime(nanos.toLong())
        // Task 43 — runtime surface-size change (screen rotation). The
        // host swaps the logical width/height when the device rotates;
        // `doFrame` feeds the new values here every frame. Re-size the
        // scene + window info so Compose re-lays-out to the new geometry
        // (otherwise the old portrait layout is drawn into the rotated
        // buffer and only a corner is visible). Only acts on an actual
        // change, so the steady state is a cheap comparison.
        if (w > 0 && h > 0) {
            val cur = realScene.size
            if (cur == null || cur.width != w || cur.height != h) {
                val sz = androidx.compose.ui.unit.IntSize(w, h)
                realScene.size = sz
                realSceneWindowInfo?.containerSize = sz
            }
        }
        canvas.clear(0xFF1A1A2E.toInt())
        realScene.render(canvas.asComposeCanvas(), nanos.toLong())
        // Drain any continuations that were queued during scene.render()
        // (mostly: withFrameNanos resumers after the frame clock sent a
        // tick). Without this they'd block the Transition.animateTo
        // loop and Material3 widgets driven by updateTransition
        // (Checkbox, DropdownMenu, …) would freeze after first toggle.
        // See WasiFrameDispatcher.kt.
        wasiFrameDispatcher.flush()
    }
}
