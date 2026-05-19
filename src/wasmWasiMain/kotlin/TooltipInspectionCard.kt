package testapp

import androidx.compose.foundation.background
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
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
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.unit.dp

/// Tooltip-on-wasi SIGILL bisect harness. Currently set to test #16
/// (TooltipBox + focusable), which DOES NOT crash, so the app boots
/// clean. Cycle through other layers below by swapping the body when
/// continuing the investigation. Full bisect table + memory pointer in
/// feedback_tooltip_sigill_wasi.md. Confirmed trigger:
///   TooltipBox + Modifier.clickable (as a composed Modifier.Node).
/// All individual pieces of clickable (gesture detector, semantics,
/// focusable, ripple) work fine inside TooltipBox when isolated.
///
/// Bisect results (16 tests):
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
///   #16 TooltipBox + focusable():               ✅  ← current
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun TooltipInspectionCard() {
    var taps by remember { mutableIntStateOf(0) }
    val interactionSource = remember { MutableInteractionSource() }
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
                "Tooltip test #16 — TooltipBox + Modifier.focusable()",
                style = MaterialTheme.typography.titleSmall,
            )
            Text("taps=$taps",
                style = MaterialTheme.typography.bodySmall)
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
                        .focusable(interactionSource = interactionSource)
                        .pointerInput(Unit) {
                            awaitEachGesture {
                                awaitFirstDown(pass = PointerEventPass.Main)
                                taps++
                            }
                        },
                    contentAlignment = Alignment.Center,
                ) {
                    Text("tap me", color = Color.Black)
                }
            }
        }
    }
}
