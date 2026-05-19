package testapp

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TooltipAnchorPosition
import androidx.compose.material3.TooltipBox
import androidx.compose.material3.TooltipDefaults
import androidx.compose.material3.rememberTooltipState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.node.ModifierNodeElement
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import org.jetbrains.skiko.wasi.wit.Canvas as WitCanvas

/// Tooltip-on-wasi SIGILL bisect harness. Step 2 of task 29 done.
///
/// Step 1 result: hand-built wrapper structure alone (scope +
/// dead-conditional-Popup + DisposableEffect) is innocent — real
/// BasicTooltipState machinery is part of the trigger.
///
/// Step 2 result: the common path of every crashing variant is
/// `BasicTooltipState.show()` (Tooltip.kt:1055) — the
/// `suspendCancellableCoroutine` inside `mutatorMutex.mutate`.
/// Two entry points: (a) enabled-clickable Press → requestFocus →
/// keyboardBehavior.onFocusChanged → state.show; (b) long-press
/// timeout → handleGestures → state.show. Disabled-clickable +
/// short tap survives because neither path is reached.
///
/// Currently set to test #28.
///
/// Bisect results so far (28 tests):
///   #1-#16 — see prior git history of this file.
///   #17 hand-built WithScope(popup=t,dispEff=t)
///       + clickable:                            ✅  (61 taps clean)
///   #18-#22 deleted as moot (base ✅).
///   #23 TooltipBox + clickable + composition
///       probe (DisposableEffect+SideEffect):    💥 ~immediate on Press
///       (only initial attach + recompose-#1
///        before crash — composition is stable.)
///   #24 hand-built wrapper + clickable + same
///       composition probe:                      ✅ (30 taps)
///       (identical attach+recompose signal as #23
///        → composition lifecycle is uninformative.)
///   #25 TooltipBox + hand-rolled
///       (pointerInput + semantics + focusable): ✅ (11 manual taps)
///       (NOT the feature combination — bug is
///        ClickableNode-specific OR a deeper path
///        triggered only by clickable's behaviour.)
///   #26 TooltipBox + clickable + custom
///       Modifier.Node lifecycle probe:          💥 on first Press
///       (probes attach once at startup; no
///        detach/reattach/update before crash.)
///   #27 TooltipBox + clickable + passive
///       pointer-event observer (Initial pass):  💥 within 10 ms of Press
///       (crash window pinpointed to Press
///        dispatch — observer logs `type=Press`
///        then SIGILL 10 ms later.)
///   #28 TooltipBox + clickable(enabled=false):  ✅ short / 💥 long  ← current
///       (short tap survives → active path needed;
///        long tap still crashes → handleGestures'
///        long-press timeout reaches state.show
///        even when clickable is disabled.)
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun TooltipInspectionCard() {
    var taps by remember { mutableIntStateOf(0) }
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant
        ),
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                "Tooltip test #28 — real TooltipBox + " +
                    "clickable(enabled=false)",
                style = MaterialTheme.typography.titleSmall,
            )
            Text(
                "taps=$taps",
                style = MaterialTheme.typography.bodySmall,
            )
            TooltipBox(
                positionProvider = TooltipDefaults
                    .rememberTooltipPositionProvider(
                        TooltipAnchorPosition.Above
                    ),
                tooltip = { Text("hello") },
                state = rememberTooltipState(),
            ) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .size(width = 200.dp, height = 80.dp)
                        .background(Color(0xFFFFAA00))
                        .clickable(enabled = false) { taps++ },
                    contentAlignment = Alignment.Center,
                ) {
                    Text("tap me", color = Color.Black)
                }
            }
        }
    }
}

/// Test #26 — Custom Modifier.Node observer. Logs `onAttach`,
/// `onDetach`, and `update` at the Modifier.Node level (which the
/// composition-side probe in tests #23/#24 cannot see). Chained on
/// either side of `Modifier.clickable` to observe how the surrounding
/// node-tree lifecycle differs between the crashing path (TooltipBox)
/// and the non-crashing path (hand-built wrapper).
private class LifecycleProbeNode(var label: String) : Modifier.Node() {
    override fun onAttach() {
        super.onAttach()
        WitCanvas.Import.logMessage("$label attach")
    }

    override fun onDetach() {
        super.onDetach()
        WitCanvas.Import.logMessage("$label detach")
    }
}

private data class LifecycleProbeElement(
    val label: String,
) : ModifierNodeElement<LifecycleProbeNode>() {
    override fun create(): LifecycleProbeNode = LifecycleProbeNode(label)

    override fun update(node: LifecycleProbeNode) {
        val prev = node.label
        node.label = label
        WitCanvas.Import.logMessage("$label update (prev=$prev)")
    }
}

private fun Modifier.lifecycleProbe(label: String): Modifier =
    this then LifecycleProbeElement(label)

/// Test #25 — Hand-rolled equivalent of `Modifier.clickable` built
/// from its constituent pieces: `pointerInput(detectTapGestures)` +
/// `semantics { role = Button; onClick { ... } }` + `focusable`.
/// Critically AVOIDS `Modifier.clickable` (and hence `ClickableElement`
/// / `ClickableNode`). If TooltipBox + this combination crashes, the
/// bug is in the feature combination; if it does not, the bug is
/// specific to ClickableElement's Modifier.Node implementation.
@Composable
private fun HandRolledClickableBox(
    onClick: () -> Unit,
) {
    val interactionSource = remember { MutableInteractionSource() }
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .size(width = 200.dp, height = 80.dp)
            .background(Color(0xFFFFAA00))
            .pointerInput(Unit) {
                detectTapGestures(onTap = { onClick() })
            }
            .semantics {
                role = Role.Button
                onClick(label = "tap me") { onClick(); true }
            }
            .focusable(interactionSource = interactionSource),
        contentAlignment = Alignment.Center,
    ) {
        Text("tap me", color = Color.Black)
    }
}

/// A `Box.clickable` with a `DisposableEffect` (logs composition
/// attach/detach) + `SideEffect` (logs each recompose) inside its
/// content. The probe sits INSIDE the composable subtree without
/// touching the modifier chain — this observes the surrounding
/// composition lifecycle of the clickable-bearing Box without
/// changing what the modifier chain looks like to Compose.
@Composable
@Suppress("unused")
private fun ProbedClickableBox(
    probeLabel: String,
    onClick: () -> Unit,
) {
    val recomposeCount = remember { intArrayOf(0) }
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .size(width = 200.dp, height = 80.dp)
            .background(Color(0xFFFFAA00))
            .clickable { onClick() },
        contentAlignment = Alignment.Center,
    ) {
        DisposableEffect(Unit) {
            WitCanvas.Import.logMessage("$probeLabel attach")
            onDispose {
                WitCanvas.Import.logMessage("$probeLabel detach")
            }
        }
        SideEffect {
            recomposeCount[0]++
            WitCanvas.Import.logMessage(
                "$probeLabel recompose #${recomposeCount[0]}"
            )
        }
        Text("tap me", color = Color.Black)
    }
}

/// Replicates BasicTooltipBox's outer wrapper structure WITH the
/// `rememberCoroutineScope()` call (matches BasicTooltip.kt:107).
/// `sentinel` is never flipped — its read inside the `if` happens on
/// every recompose, modelling the always-recomposed snapshot read of
/// `state.isVisible` in BasicTooltipBox. Used by test #17 / #24.
@Composable
@Suppress("unused")
private fun HandBuiltTooltipWrapperWithScope(
    includePopupSlot: Boolean,
    includeDisposableEffect: Boolean,
    content: @Composable () -> Unit,
) {
    @Suppress("UNUSED_VARIABLE")
    val scope = rememberCoroutineScope()
    val sentinel = remember { mutableStateOf(false) }
    Box {
        if (includePopupSlot && sentinel.value) {
            Popup(onDismissRequest = {}) { Text("dead") }
        }
        content()
    }
    if (includeDisposableEffect) {
        DisposableEffect(sentinel) { onDispose { } }
    }
}
