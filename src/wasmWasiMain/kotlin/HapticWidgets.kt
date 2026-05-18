/*
 * HapticWidgets.kt — a tiny design-system layer on top of Material3
 * that wires `LocalHapticFeedback` into the interactions Material3
 * itself doesn't auto-haptic (buttons, checkboxes, switches, radio
 * buttons, clickable rows).
 *
 * The bridge in `RealComposeApp.kt::buildRealComposeScene` provides
 * `LocalHapticFeedback` as our `WasiHapticFeedback`. Material3's
 * own auto-haptic widgets (Slider w/ steps, DatePicker drum,
 * BasicTextField selection) already pick that up. THIS file covers
 * everything else.
 *
 * The single source of truth for "is haptic on right now?" is the
 * `LocalHapticEnabled` composition local — set once at the
 * `MaterialDemoApp` level from the user-facing toggle Switch and
 * read by every Haptic* widget when deciding whether to fire.
 *
 * The toggle Switch in the demo itself does NOT use HapticSwitch —
 * it would be weird for the "Enable haptic" control to buzz while
 * being flipped.
 */
@file:Suppress("unused")

package testapp

import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.Interaction
import androidx.compose.foundation.selection.selectable
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonColors
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ButtonElevation
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CheckboxColors
import androidx.compose.material3.CheckboxDefaults
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonColors
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.MenuItemColors
import androidx.compose.material3.MenuDefaults
import androidx.compose.material3.RadioButton
import androidx.compose.material3.RadioButtonColors
import androidx.compose.material3.RadioButtonDefaults
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchColors
import androidx.compose.material3.SwitchDefaults
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.ProvidableCompositionLocal
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.painter.Painter
import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.Role

/**
 * Whether haptic feedback fires on Haptic* widget interactions.
 *
 * Defaults to `true`. The demo flips this via
 * `CompositionLocalProvider(LocalHapticEnabled provides ...)` driven
 * by the user-facing "Enable haptic" Switch.
 *
 * Note: this only gates Haptic* widgets defined in this file. The
 * underlying `LocalHapticFeedback` provider (our `WasiHapticFeedback`)
 * remains installed unconditionally, so Material3's auto-haptic
 * widgets (Slider w/ steps, DatePicker, …) continue to buzz
 * regardless. If you need to gate THOSE too, gate by replacing the
 * `LocalHapticFeedback` provider with a no-op instead.
 */
val LocalHapticEnabled: ProvidableCompositionLocal<Boolean> =
    compositionLocalOf { true }

/**
 * No-op `HapticFeedback` — used by [HapticScope] to swap out the
 * real `WasiHapticFeedback` when [LocalHapticEnabled] flips off,
 * so Material3's own auto-haptic widgets (Slider w/ steps,
 * DatePicker drum, TextField selection) also stop buzzing
 * without needing per-widget gating.
 */
private object NoOpHapticFeedback : HapticFeedback {
    override fun performHapticFeedback(hapticFeedbackType: HapticFeedbackType) {}
}

/**
 * Single-source-of-truth gate for haptic feedback within [content].
 *
 * Sets [LocalHapticEnabled] to [enabled], and — crucially — also
 * swaps [LocalHapticFeedback] for a no-op impl when [enabled] is
 * false. The latter means Material3's own auto-haptic widgets
 * (Slider with `steps > 0`, DatePicker drum, BasicTextField
 * selection long-press) honor the user toggle too — without us
 * having to fork or wrap each of them.
 *
 * Usage:
 * ```
 * var hapticEnabled by remember { mutableStateOf(true) }
 * HapticScope(enabled = hapticEnabled) {
 *     ...                                // children
 *     HapticSwitch(checked = hapticEnabled,
 *                  onCheckedChange = { hapticEnabled = it })
 *     ...
 * }
 * ```
 */
@Composable
fun HapticScope(
    enabled: Boolean,
    content: @Composable () -> Unit,
) {
    val realHaptic = LocalHapticFeedback.current
    val effectiveHaptic: HapticFeedback =
        if (enabled) realHaptic else NoOpHapticFeedback
    androidx.compose.runtime.CompositionLocalProvider(
        LocalHapticEnabled provides enabled,
        LocalHapticFeedback provides effectiveHaptic,
    ) {
        content()
    }
}

/**
 * Fire [feedback] on [haptic] iff [enabled]. Centralized so every
 * Haptic* widget has the same gating policy.
 */
private fun maybeFire(
    enabled: Boolean,
    haptic: HapticFeedback,
    feedback: HapticFeedbackType,
) {
    if (enabled) haptic.performHapticFeedback(feedback)
}

/**
 * Toggle-shaped feedback: ToggleOn when transitioning to true,
 * ToggleOff otherwise. Saves duplicating the conditional in every
 * Switch/Checkbox call site.
 */
private fun toggleFeedback(newState: Boolean): HapticFeedbackType =
    if (newState) HapticFeedbackType.ToggleOn else HapticFeedbackType.ToggleOff

// ─── Buttons ───────────────────────────────────────────────────────

/**
 * Drop-in replacement for `androidx.compose.material3.Button`.
 *
 * Adds a `performHapticFeedback(feedback)` call before invoking
 * [onClick], gated by [LocalHapticEnabled]. Default feedback is
 * [HapticFeedbackType.Confirm] → maps to CLICK + MEDIUM via
 * [WasiHapticFeedback].
 *
 * Parameters that aren't haptic-related are forwarded to Material3
 * `Button` unchanged.
 */
@Composable
fun HapticButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    feedback: HapticFeedbackType = HapticFeedbackType.Confirm,
    shape: Shape = ButtonDefaults.shape,
    colors: ButtonColors = ButtonDefaults.buttonColors(),
    elevation: ButtonElevation? = ButtonDefaults.buttonElevation(),
    contentPadding: PaddingValues = ButtonDefaults.ContentPadding,
    content: @Composable RowScope.() -> Unit,
) {
    val haptic = LocalHapticFeedback.current
    val hapticEnabled = LocalHapticEnabled.current
    Button(
        onClick = {
            maybeFire(hapticEnabled, haptic, feedback)
            onClick()
        },
        modifier = modifier,
        enabled = enabled,
        shape = shape,
        colors = colors,
        elevation = elevation,
        contentPadding = contentPadding,
        content = content,
    )
}

/**
 * Drop-in replacement for `IconButton`. Same haptic-on-click
 * semantics as [HapticButton].
 */
@Composable
fun HapticIconButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    feedback: HapticFeedbackType = HapticFeedbackType.Confirm,
    colors: IconButtonColors = IconButtonDefaults.iconButtonColors(),
    content: @Composable () -> Unit,
) {
    val haptic = LocalHapticFeedback.current
    val hapticEnabled = LocalHapticEnabled.current
    IconButton(
        onClick = {
            maybeFire(hapticEnabled, haptic, feedback)
            onClick()
        },
        modifier = modifier,
        enabled = enabled,
        colors = colors,
        content = content,
    )
}

// ─── Selection / toggle widgets ────────────────────────────────────

/**
 * Drop-in replacement for `Checkbox` that haptics on state change.
 * Fires `ToggleOn` when checking, `ToggleOff` when unchecking — both
 * map to CLICK + MEDIUM (light buzz) via [WasiHapticFeedback].
 */
@Composable
fun HapticCheckbox(
    checked: Boolean,
    onCheckedChange: ((Boolean) -> Unit)?,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    colors: CheckboxColors = CheckboxDefaults.colors(),
) {
    val haptic = LocalHapticFeedback.current
    val hapticEnabled = LocalHapticEnabled.current
    Checkbox(
        checked = checked,
        onCheckedChange = onCheckedChange?.let { cb ->
            { new ->
                maybeFire(hapticEnabled, haptic, toggleFeedback(new))
                cb(new)
            }
        },
        modifier = modifier,
        enabled = enabled,
        colors = colors,
    )
}

/**
 * Drop-in replacement for `RadioButton`. Fires `Confirm` on
 * selection. (No haptic on deselect since RadioButton.onClick
 * isn't called for the row being deselected — that's implicit
 * when another in the group is picked.)
 */
@Composable
fun HapticRadioButton(
    selected: Boolean,
    onClick: (() -> Unit)?,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    feedback: HapticFeedbackType = HapticFeedbackType.Confirm,
    colors: RadioButtonColors = RadioButtonDefaults.colors(),
) {
    val haptic = LocalHapticFeedback.current
    val hapticEnabled = LocalHapticEnabled.current
    RadioButton(
        selected = selected,
        onClick = onClick?.let { cb ->
            {
                maybeFire(hapticEnabled, haptic, feedback)
                cb()
            }
        },
        modifier = modifier,
        enabled = enabled,
        colors = colors,
    )
}

/**
 * Drop-in replacement for `Switch`. Fires `ToggleOn`/`ToggleOff`
 * depending on direction. Note: the "Enable haptic" toggle in the
 * demo deliberately uses the plain Material3 `Switch` (not this) so
 * the haptic-on-toggle setting doesn't itself buzz while being
 * flipped — that would be confusing UX.
 */
@Composable
fun HapticSwitch(
    checked: Boolean,
    onCheckedChange: ((Boolean) -> Unit)?,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    colors: SwitchColors = SwitchDefaults.colors(),
) {
    val haptic = LocalHapticFeedback.current
    val hapticEnabled = LocalHapticEnabled.current
    Switch(
        checked = checked,
        onCheckedChange = onCheckedChange?.let { cb ->
            { new ->
                maybeFire(hapticEnabled, haptic, toggleFeedback(new))
                cb(new)
            }
        },
        modifier = modifier,
        enabled = enabled,
        colors = colors,
    )
}

/**
 * Drop-in replacement for `AssistChip`. Non-toggling; fires
 * `Confirm` on tap.
 */
@Composable
fun HapticAssistChip(
    onClick: () -> Unit,
    label: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    feedback: HapticFeedbackType = HapticFeedbackType.Confirm,
    leadingIcon: @Composable (() -> Unit)? = null,
    trailingIcon: @Composable (() -> Unit)? = null,
) {
    val haptic = LocalHapticFeedback.current
    val hapticEnabled = LocalHapticEnabled.current
    androidx.compose.material3.AssistChip(
        onClick = {
            maybeFire(hapticEnabled, haptic, feedback)
            onClick()
        },
        label = label,
        modifier = modifier,
        enabled = enabled,
        leadingIcon = leadingIcon,
        trailingIcon = trailingIcon,
    )
}

/**
 * Drop-in replacement for `SuggestionChip`. Same shape as
 * AssistChip: non-toggling, fires `Confirm` on tap.
 */
@Composable
fun HapticSuggestionChip(
    onClick: () -> Unit,
    label: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    feedback: HapticFeedbackType = HapticFeedbackType.Confirm,
    icon: @Composable (() -> Unit)? = null,
) {
    val haptic = LocalHapticFeedback.current
    val hapticEnabled = LocalHapticEnabled.current
    androidx.compose.material3.SuggestionChip(
        onClick = {
            maybeFire(hapticEnabled, haptic, feedback)
            onClick()
        },
        label = label,
        modifier = modifier,
        enabled = enabled,
        icon = icon,
    )
}

/**
 * Drop-in replacement for `InputChip`. Toggle-shaped (like
 * FilterChip): ToggleOn/ToggleOff on flip.
 */
@Composable
fun HapticInputChip(
    selected: Boolean,
    onClick: () -> Unit,
    label: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    avatar: @Composable (() -> Unit)? = null,
    leadingIcon: @Composable (() -> Unit)? = null,
    trailingIcon: @Composable (() -> Unit)? = null,
) {
    val haptic = LocalHapticFeedback.current
    val hapticEnabled = LocalHapticEnabled.current
    androidx.compose.material3.InputChip(
        selected = selected,
        onClick = {
            maybeFire(hapticEnabled, haptic, toggleFeedback(!selected))
            onClick()
        },
        label = label,
        modifier = modifier,
        enabled = enabled,
        avatar = avatar,
        leadingIcon = leadingIcon,
        trailingIcon = trailingIcon,
    )
}

/**
 * Drop-in replacement for `FloatingActionButton`. Fires `Confirm`
 * by default; pass `feedback = LongPress` for a stronger primary-action buzz.
 */
@Composable
fun HapticFloatingActionButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    feedback: HapticFeedbackType = HapticFeedbackType.Confirm,
    content: @Composable () -> Unit,
) {
    val haptic = LocalHapticFeedback.current
    val hapticEnabled = LocalHapticEnabled.current
    androidx.compose.material3.FloatingActionButton(
        onClick = {
            maybeFire(hapticEnabled, haptic, feedback)
            onClick()
        },
        modifier = modifier,
        content = content,
    )
}

/**
 * Drop-in replacement for `ExtendedFloatingActionButton`.
 */
@Composable
fun HapticExtendedFloatingActionButton(
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    feedback: HapticFeedbackType = HapticFeedbackType.Confirm,
    icon: @Composable () -> Unit = {},
    text: @Composable () -> Unit,
) {
    val haptic = LocalHapticFeedback.current
    val hapticEnabled = LocalHapticEnabled.current
    androidx.compose.material3.ExtendedFloatingActionButton(
        onClick = {
            maybeFire(hapticEnabled, haptic, feedback)
            onClick()
        },
        modifier = modifier,
        icon = icon,
        text = text,
    )
}

/**
 * Drop-in replacement for `FilterChip`. Toggle-shaped: fires
 * `ToggleOn` when transitioning to selected, `ToggleOff` otherwise.
 *
 * `selected` here is the CURRENT state; after the user clicks,
 * intent is to flip to `!selected`, so we use that for the haptic
 * direction (a press that selects → ToggleOn).
 */
@Composable
fun HapticFilterChip(
    selected: Boolean,
    onClick: () -> Unit,
    label: @Composable () -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    leadingIcon: @Composable (() -> Unit)? = null,
    trailingIcon: @Composable (() -> Unit)? = null,
) {
    val haptic = LocalHapticFeedback.current
    val hapticEnabled = LocalHapticEnabled.current
    androidx.compose.material3.FilterChip(
        selected = selected,
        onClick = {
            maybeFire(hapticEnabled, haptic, toggleFeedback(!selected))
            onClick()
        },
        label = label,
        modifier = modifier,
        enabled = enabled,
        leadingIcon = leadingIcon,
        trailingIcon = trailingIcon,
    )
}

/**
 * Drop-in replacement for `Slider` that fires `SegmentTick` haptic
 * each time the slider's value crosses a step boundary.
 *
 * compose-multiplatform-core's current Material3 Slider has no
 * built-in haptic call (the auto-haptic-on-step-crossing feature
 * lives in a newer upstream Material3 we haven't yet bumped to).
 * This wrapper computes the discrete step index from the value
 * and fires haptic on every change of that index.
 *
 * Requires [steps] > 0; for a continuous slider, just use vanilla
 * `Slider` — there are no semantic step boundaries to buzz on.
 */
@Composable
fun HapticSlider(
    value: Float,
    onValueChange: (Float) -> Unit,
    steps: Int,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
    valueRange: ClosedFloatingPointRange<Float> = 0f..1f,
    feedback: HapticFeedbackType = HapticFeedbackType.SegmentTick,
    onValueChangeFinished: (() -> Unit)? = null,
) {
    require(steps > 0) {
        "HapticSlider needs steps > 0; for continuous use plain Slider"
    }
    val haptic = LocalHapticFeedback.current
    val hapticEnabled = LocalHapticEnabled.current

    // Track the LAST step index we sat at so we only fire on
    // crossings (not on every onValueChange tick during drag).
    val numPositions = steps + 1
    val rangeSpan = valueRange.endInclusive - valueRange.start
    fun toStep(v: Float): Int {
        val frac = ((v - valueRange.start) / rangeSpan).coerceIn(0f, 1f)
        return (frac * numPositions).toInt().coerceIn(0, numPositions)
    }
    val lastStep = remember { androidx.compose.runtime.mutableIntStateOf(toStep(value)) }

    androidx.compose.material3.Slider(
        value = value,
        onValueChange = { newV ->
            val newStep = toStep(newV)
            if (newStep != lastStep.intValue) {
                maybeFire(hapticEnabled, haptic, feedback)
                lastStep.intValue = newStep
            }
            onValueChange(newV)
        },
        modifier = modifier,
        enabled = enabled,
        valueRange = valueRange,
        steps = steps,
        onValueChangeFinished = onValueChangeFinished,
    )
}

/**
 * Drop-in replacement for `DropdownMenuItem` that haptics on
 * selection. Fires `Confirm`.
 */
@Composable
fun HapticDropdownMenuItem(
    text: @Composable () -> Unit,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    leadingIcon: @Composable (() -> Unit)? = null,
    trailingIcon: @Composable (() -> Unit)? = null,
    enabled: Boolean = true,
    feedback: HapticFeedbackType = HapticFeedbackType.Confirm,
    colors: MenuItemColors = MenuDefaults.itemColors(),
    contentPadding: PaddingValues = MenuDefaults.DropdownMenuItemContentPadding,
) {
    val haptic = LocalHapticFeedback.current
    val hapticEnabled = LocalHapticEnabled.current
    DropdownMenuItem(
        text = text,
        onClick = {
            maybeFire(hapticEnabled, haptic, feedback)
            onClick()
        },
        modifier = modifier,
        leadingIcon = leadingIcon,
        trailingIcon = trailingIcon,
        enabled = enabled,
        colors = colors,
        contentPadding = contentPadding,
    )
}

// ─── Modifier helpers ─────────────────────────────────────────────

/**
 * Like `Modifier.clickable { ... }` but fires haptic feedback before
 * invoking the click handler, gated by [LocalHapticEnabled].
 *
 * Useful for ad-hoc tappable rows (Card surfaces, list items, etc.)
 * that aren't backed by a specific Material3 component.
 */
fun Modifier.hapticClickable(
    enabled: Boolean = true,
    feedback: HapticFeedbackType = HapticFeedbackType.Confirm,
    onClickLabel: String? = null,
    role: Role? = null,
    onClick: () -> Unit,
): Modifier = composed {
    val haptic = LocalHapticFeedback.current
    val hapticEnabled = LocalHapticEnabled.current
    this.clickable(
        enabled = enabled,
        onClickLabel = onClickLabel,
        role = role,
    ) {
        maybeFire(hapticEnabled, haptic, feedback)
        onClick()
    }
}

/**
 * Like `Modifier.selectable(...)` but fires haptic on (re-)selection.
 * Used by row-wrapped RadioButton groups where the click target is
 * the whole row, not the RadioButton itself.
 */
fun Modifier.hapticSelectable(
    selected: Boolean,
    enabled: Boolean = true,
    role: Role? = null,
    feedback: HapticFeedbackType = HapticFeedbackType.Confirm,
    onClick: () -> Unit,
): Modifier = composed {
    val haptic = LocalHapticFeedback.current
    val hapticEnabled = LocalHapticEnabled.current
    this.selectable(
        selected = selected,
        enabled = enabled,
        role = role,
    ) {
        maybeFire(hapticEnabled, haptic, feedback)
        onClick()
    }
}
