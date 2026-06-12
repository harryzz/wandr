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
import org.jetbrains.skiko.wasi.shell.Clipboard as WitClipboard
import wandr.platform.Haptics as WitHaptics
import wandr.platform.Power as WitPower
import wandr.platform.Sensors as WitSensors
import wandr.platform.Thermal as WitThermal
import org.jetbrains.skiko.wasi.shell.Locale as WitLocale
import wandr.platform.PointerIcon as WitPointerIcon
import org.jetbrains.skiko.wasi.shell.TextSegmentation as WitTextSeg
import org.jetbrains.skiko.wasi.shell.Metrics as WitWindow
import wandr.platform.Display as WitDisplay
import testapp.compose.App
import testapp.compose.WasiComposeScene

fun main() {
    // Compose smoke tests removed — they belonged to wandr-app's
    // multi-card demo, not to a minimal IME guest. Re-add them
    // if/when this app grows beyond the keyboard.

    val density = WitWindow.Import.getDensity()
    val fontScale = WitWindow.Import.getFontScale()
    val dpi = WitWindow.Import.getDpi()
    logMessage(
        "android-window smoke: density=${density} px/dp, fontScale=${fontScale}, dpi=${dpi}"
    )

    val h1 = WasiScheduler.schedule(800u) {
        logMessage("android-scheduler smoke: fired @800ms ✓")
    }
    val h2 = WasiScheduler.schedule(200u) {
        logMessage("android-scheduler smoke: BUG — cancelled task still fired")
    }
    WasiScheduler.cancel(h2)
    logMessage(
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
    logMessage(
        "text-segmentation smoke: word-bounds=${wordBoundaries}, " +
        "sentences-end-at=[${sentenceFirst}, ${sentenceSecond}], " +
        "grapheme-≥3=${graphemeAt3}, prev-word-≤16=${prevWordBefore16}, " +
        "total-utf8-bytes=${sampleLen}"
    )

    // android-input-v2 smoke: log first 5 enriched events that arrive.
    var v2Count = 0
    // input-002 smoke: the first 5 union events are logged from the scene
    // dispatch handler registered below (single handler slot).

    // android-lifecycle smoke: log current state + register an observer.
    logMessage(
        "android-lifecycle smoke: currentState=${WasiLifecycle.currentState()} at boot"
    )
    WasiLifecycle.addObserver { state ->
        logMessage("android-lifecycle smoke: → ${state}")
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
    logMessage(
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
    logMessage(
        "android-sensors smoke: ${sensors.size} sensors; accel handle=${accelInfo?.handle ?: 0u}"
    )
    if (accelInfo != null) {
        WitSensors.Import.enable(accelInfo.handle, 50u)
        WasiScheduler.schedule(200u) {
            val s = WitSensors.Import.pollLatest(accelInfo.handle)
            logMessage(
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
    logMessage(
        "android-haptics smoke: perform(TAP)=${tapOk}, vibrateMs(50)=${vibrateOk}"
    )

    // android-audio smoke: open a mono PCM-f32 track at 48 kHz, push a
    // 200 ms 440 Hz sine into its ring buffer, start playback. Backed by
    // android.media.IAAudioService ("media.aaudio") via rsbinder (task
    // 21). Expected: a short beep on the device speaker when the app
    // launches. createTrack returns 0 if media.aaudio is unavailable
    // (SELinux denial, or service down) — in that case we log + skip.
    //
    // Buffer-capacity caveat: AAudio's down-data ring on a Pixel 2 XL is
    // typically a few hundred frames (~10 ms). writePcmF32 will return
    // the number of frames that actually fit, often less than the 9600
    // we asked for; the rest is dropped silently in this smoke test. A
    // production path would schedule per-frame top-ups; for "does the
    // pipeline work?" the partial beep is enough.
    // Stereo + 48 kHz: the only MMAP-supported config on Pixel 2 XL per the
    // service's "suggested channel_mask=0x3" hint in earlier attempts. Mono
    // PCM-f32 was refused without an AudioFlinger fallback; stereo lets the
    // MMAP path succeed. We duplicate the sine into both channels.
    val audioTrack = wandr.platform.Pcm.Playback.open(wandr.platform.Pcm.StreamConfig(
        sampleRate     = 48000u,
        channelLayout  = wandr.platform.Pcm.ChannelLayout.STEREO,
        format         = wandr.platform.Pcm.Format.PCM_F32,
        class_         = wandr.platform.Pcm.StreamClass.MEDIA,
    )).getOrNull()
    if (audioTrack != null) {
        val sr     = 48000
        val freq   = 440.0
        val frames = 9600  // 200 ms
        val amp    = 0.3f  // -10 dBFS — comfortable, not piercing
        val twoPi  = 2.0 * kotlin.math.PI
        val samples = ArrayList<Float>(frames * 2)  // interleaved L,R
        for (i in 0 until frames) {
            val v = (kotlin.math.sin(twoPi * freq * i / sr) * amp).toFloat()
            samples.add(v)  // L
            samples.add(v)  // R
        }
        // Standard AAudio order: pre-fill the ring, THEN start playback.
        val written = audioTrack.write(samples)
        val started = audioTrack.start().isSuccess
        val pending = audioTrack.bufferedFrames()
        logMessage(
            "audio smoke: wrote=${written}/${frames} " +
            "frames started=${started} buffered=${pending} (expect a brief beep)"
        )
    } else {
        logMessage(
            "audio smoke: playback.open failed — media.aaudio " +
            "unavailable, SELinux denial, or config rejected"
        )
    }

    // android-locale smoke: read user's locale, time format, direction.
    val loc       = WitLocale.Import.primaryLocale()
    val is24      = WitLocale.Import.isTwentyFourHourFormat()
    val direction = WitLocale.Import.getLayoutDirection()
    logMessage(
        "android-locale smoke: primary=${loc}, 24h=${is24}, direction=${direction}"
    )

    // android-clipboard smoke: starts empty, write, read back, clear, read.
    val before = WitClipboard.Import.hasText()
    WitClipboard.Import.setText("Hello, clipboard! 复制粘贴 ⌘C")
    val mid    = WitClipboard.Import.getText()
    val hasMid = WitClipboard.Import.hasText()
    WitClipboard.Import.clear()
    val after  = WitClipboard.Import.hasText()
    logMessage(
        "android-clipboard smoke: before=hasText=${before}, " +
        "after-set: text=\"${mid}\" hasText=${hasMid}, " +
        "after-clear: hasText=${after}"
    )

    // android-pointer-icon smoke: must not crash on touch-only device.
    WitPointerIcon.Import.set(WitPointerIcon.Kind.TEXT)
    WitPointerIcon.Import.set(WitPointerIcon.Kind.HAND)
    WitPointerIcon.Import.set(WitPointerIcon.Kind.DEFAULT)
    logMessage("android-pointer-icon smoke: 3 calls completed (no-op on touch)")

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
    // Boot-time surface size (no frame is active yet): the app content
    // area from wandr:chrome/display; the per-frame resize path below
    // corrects any drift on the first frame.
    val bootSize = WitDisplay.Import.contentSize()
    val widthPx  = bootSize.width.toInt()
    val heightPx = bootSize.height.toInt()
    val realScene = buildRealComposeScene(widthPx, heightPx, density)
    logMessage(
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

    // Pointer events arrive EXCLUSIVELY through the wasi:input-handlers
    // 0.0.2 export (skiko WasiInput) — the legacy SkikoInputDelegate leg is
    // retired with the my:skiko-gfx renderer export.
    WasiInput.setPointerHandler { ev ->
        if (v2Count < 5) {
            v2Count++
            logMessage(
                "input-002 smoke #${v2Count}: id=${ev.id} kind=${ev.kind} dev=${ev.device} " +
                "@(${ev.x.toInt()},${ev.y.toInt()}) pressure=${ev.pressure}"
            )
        }
        val type = androidx.compose.ui.input.pointer.PointerEventType
        val k = org.jetbrains.skiko.wasi.shell.PointerHandler.Kind
        val evtType = when (ev.kind) {
            k.DOWN   -> type.Press
            k.UP, k.CANCEL -> type.Release
            k.MOVE   -> type.Move
            k.SCROLL -> type.Scroll
            k.ENTER  -> type.Enter
            k.LEAVE  -> type.Exit
        }
        realScene.sendPointerEvent(
            eventType = evtType,
            position = androidx.compose.ui.geometry.Offset(ev.x, ev.y),
            type = androidx.compose.ui.input.pointer.PointerType.Touch,
        )
        // Drain the dispatcher after EVERY pointer event so the
        // suspending pointer-input coroutines get to await the NEXT
        // event before it arrives (see wandr-app's note).
        wasiFrameDispatcher.flush()
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
    org.jetbrains.skiko.wasi.WasiInput.setKeyHandler { ev ->
        val codePoint = firstCodePoint(ev.text)
        val keyId = w3cCodeToKeyId(ev.code)
        @OptIn(androidx.compose.ui.InternalComposeUiApi::class)
        val type = if (ev.down) {
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
        // Task 62 — overlay rotation. When the device rotates the host
        // flips the IME's anchored rect (bottom strip → side strip) and
        // swaps the logical width/height; `doFrame` feeds the new values
        // here every frame. Re-size the scene + window info so the keyboard
        // re-lays-out to the new geometry (its rows use weight(1f), so they
        // fill whatever height is given). Without this the scene keeps its
        // startup (portrait 1200-px) size and overflows / clips the rotated
        // surface. Mirrors wandr-app's task-43 fix; only acts on a change.
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

// ── key-event helpers (moved from the retired InputHandlerExports.kt) ───────

internal fun w3cCodeToKeyId(code: String): UInt = when (code) {
    "Backspace" -> 8u
    "Tab" -> 9u
    "Enter", "NumpadEnter" -> 13u
    "Escape" -> 27u
    "Space" -> 32u
    "PageUp" -> 33u
    "PageDown" -> 34u
    "End" -> 35u
    "Home" -> 36u
    "ArrowLeft" -> 37u
    "ArrowUp" -> 38u
    "ArrowRight" -> 39u
    "ArrowDown" -> 40u
    "Insert" -> 45u
    "Delete" -> 46u
    else -> 0u
}

internal fun firstCodePoint(s: String): UInt {
    if (s.isEmpty()) return 0u
    val c0 = s[0]
    return if (c0.isHighSurrogate() && s.length > 1 && s[1].isLowSurrogate()) {
        ((((c0.code - 0xD800) shl 10) or (s[1].code - 0xDC00)) + 0x10000).toUInt()
    } else {
        c0.code.toUInt()
    }
}
