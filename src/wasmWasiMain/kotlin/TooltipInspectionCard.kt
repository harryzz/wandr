package testapp

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup

/// Tooltip-on-wasi SIGILL bisect harness. Step 1 of task 29 — hand-built
/// BasicTooltipBox wrapper bisect. Currently set to test #17.
/// Full bisect context: tasks/29-tooltip-sigill-bisect.md and the
/// feedback_tooltip_sigill_wasi memory.
///
/// Confirmed trigger from tests #1-#16:
///   TooltipBox + Modifier.clickable (as a composed Modifier.Node).
///
/// Step 1 hypothesis: BasicTooltipBox's wrapper structure (scope +
/// conditional-Popup-slot + DisposableEffect) around clickable is what
/// trips it. Three suspect pieces, six new layers (#17-#22).
///
/// Bisect results so far (22 tests):
///   #1  LocalInspectionMode wrap:               💥
///   #2  enableUserInput = false:                ✅
///   #3  plain Popup on tap:                     ✅
///   #4  bare while(true) awaitPointerEvent:     ✅
///   #5  awaitEachGesture + awaitFirstDown +
///       withTimeoutOrNull(500ms) — long-press:  ✅
///   #6  both handleGestures pointerInputs
///       chained, bare Box:                      ✅
///   #7  anchorSemantics + onLongClick alone:    ✅
///   #8  handleGestures + anchorSemantics
///       combined on bare Box:                   ✅
///   #9  real TooltipBox + Box+Text only:        ✅
///   #10 real TooltipBox + IconButton{Text}:     💥
///   #11 real TooltipBox + Box.clickable{}:      💥
///   #12 hand-built modifier chain + clickable:  ✅
///   #13 TooltipBox + clickable(indication=null):💥
///   #14 TooltipBox + detectTapGestures:         ✅
///   #15 TooltipBox + semantics(role+onClick):   ✅
///   #16 TooltipBox + focusable():               ✅
///   #17 hand-built WithScope(popup=t,dispEff=t)
///       + clickable:                            ✅  ← current
///       (61 taps clean, ~45 s soak — proves the
///        wrapper structure alone is insufficient.
///        Real BasicTooltipState must be part of
///        the trigger. #18-#22 moot — would all
///        be ✅ trivially from a non-crashing base.
///        Step 2 proceeds as originally scoped:
///        ClickableNode instrumentation against the
///        real TooltipBox path.)
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
                "Tooltip test #17 — hand-built wrapper " +
                    "(scope+Popup-slot+DispEff) + clickable",
                style = MaterialTheme.typography.titleSmall,
            )
            Text(
                "taps=$taps",
                style = MaterialTheme.typography.bodySmall,
            )
            HandBuiltTooltipWrapperWithScope(
                includePopupSlot = true,
                includeDisposableEffect = true,
            ) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .size(width = 200.dp, height = 80.dp)
                        .background(Color(0xFFFFAA00))
                        .clickable { taps++ },
                    contentAlignment = Alignment.Center,
                ) {
                    Text("tap me", color = Color.Black)
                }
            }
        }
    }
}

/// Replicates BasicTooltipBox's outer wrapper structure WITH the
/// `rememberCoroutineScope()` call (matches BasicTooltip.kt:107).
/// `sentinel` is never flipped — its read inside the `if` happens on
/// every recompose, modelling the always-recomposed snapshot read of
/// `state.isVisible` in BasicTooltipBox.
@Composable
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

/// Same as `HandBuiltTooltipWrapperWithScope` but omits the
/// `rememberCoroutineScope()` call entirely. Kept as a separate
/// function (rather than a runtime flag) to keep Compose groups stable
/// across layers — see plan §"Risks".
@Composable
@Suppress("unused")
private fun HandBuiltTooltipWrapperNoScope(
    includePopupSlot: Boolean,
    includeDisposableEffect: Boolean,
    content: @Composable () -> Unit,
) {
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
