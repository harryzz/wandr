@file:OptIn(kotlin.wasm.unsafe.UnsafeWasmMemoryApi::class)

package testapp

import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import org.jetbrains.skiko.wasi.wit.Haptics as WitHaptics

/**
 * Bridge from Compose's [HapticFeedback] interface to the host's WIT
 * `haptics` interface (backed by android.hardware.vibrator.IVibrator
 * via rsbinder — see task 16).
 *
 * Compose's [HapticFeedbackType] has 13 named variants (Confirm,
 * ContextClick, GestureEnd, GestureThresholdActivate, KeyboardTap,
 * LongPress, Reject, SegmentFrequentTick, SegmentTick, TextHandleMove,
 * ToggleOn, ToggleOff, VirtualKey). The WIT enum has 5 (Tap,
 * LongPress, VirtualKey, Click, DoubleClick). We pick a coarse mapping
 * that fits each Compose type into the closest WIT bucket; finer-
 * grained effects can be added by extending the WIT enum + host
 * `haptics_impl.rs` mapping to additional AIDL `Effect` constants.
 *
 * Upstream's [androidx.compose.ui.platform.DefaultHapticFeedback] for
 * the skiko target is a literal no-op (`// TODO(demin): implement
 * HapticFeedback`); installing this bridge via `CompositionLocalProvider
 * (LocalHapticFeedback provides ...)` at the scene root replaces it
 * for every widget in the tree.
 */
class WasiHapticFeedback : HapticFeedback {
    override fun performHapticFeedback(hapticFeedbackType: HapticFeedbackType) {
        // Compose uses an opaque `value: Int` inside HapticFeedbackType
        // (see HapticFeedbackType.skiko.kt — ordinals 0..12). Match the
        // public PlatformHapticFeedbackType references rather than
        // pattern-matching on the int directly, so a future upstream
        // reorder doesn't silently invert our mapping.
        val witFeedback = when (hapticFeedbackType) {
            HapticFeedbackType.LongPress           -> WitHaptics.Feedback.LONG_PRESS
            HapticFeedbackType.VirtualKey          -> WitHaptics.Feedback.VIRTUAL_KEY
            HapticFeedbackType.KeyboardTap         -> WitHaptics.Feedback.TAP
            HapticFeedbackType.TextHandleMove      -> WitHaptics.Feedback.TAP
            HapticFeedbackType.SegmentTick         -> WitHaptics.Feedback.TAP
            HapticFeedbackType.SegmentFrequentTick -> WitHaptics.Feedback.TAP
            HapticFeedbackType.GestureEnd          -> WitHaptics.Feedback.CLICK
            HapticFeedbackType.GestureThresholdActivate -> WitHaptics.Feedback.CLICK
            HapticFeedbackType.ContextClick        -> WitHaptics.Feedback.CLICK
            HapticFeedbackType.Confirm             -> WitHaptics.Feedback.CLICK
            HapticFeedbackType.ToggleOn            -> WitHaptics.Feedback.CLICK
            HapticFeedbackType.ToggleOff           -> WitHaptics.Feedback.CLICK
            HapticFeedbackType.Reject              -> WitHaptics.Feedback.DOUBLE_CLICK
            else                                   -> WitHaptics.Feedback.CLICK
        }
        WitHaptics.Import.perform(witFeedback)
    }
}
