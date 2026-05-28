package testapp

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.PlainTooltip
import androidx.compose.material3.Text
import androidx.compose.material3.TooltipAnchorPosition
import androidx.compose.material3.TooltipBox
import androidx.compose.material3.TooltipDefaults
import androidx.compose.material3.rememberTooltipState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

/// Plain Material3 `TooltipBox` demo — long-press the button to show
/// the tooltip. The wasi Tooltip SIGILL (kotlinx `Delay` path →
/// adapter State corruption) was resolved in task 30 via the KT-86415
/// stdlib fix; this card is a normal feature demo and a regression
/// check that the long-press → `BasicTooltipState.show()` path stays
/// healthy.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun TooltipCard() {
    Card(modifier = Modifier.fillMaxWidth()) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text("Tooltip", style = MaterialTheme.typography.titleMedium)
            TooltipBox(
                positionProvider =
                    TooltipDefaults.rememberTooltipPositionProvider(
                        TooltipAnchorPosition.Above
                    ),
                tooltip = { PlainTooltip { Text("Long-pressed!") } },
                state = rememberTooltipState(),
            ) {
                Button(onClick = {}) {
                    Text("Long-press me")
                }
            }
        }
    }
}
