package testapp

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.add
import androidx.compose.foundation.layout.calculateEndPadding
import androidx.compose.foundation.layout.calculateStartPadding
import androidx.compose.foundation.layout.exclude
import androidx.compose.foundation.layout.union
import androidx.compose.ui.Alignment
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import org.jetbrains.skiko.wasi.wit.Canvas as WitCanvas

/**
 * Foundation-layout smoke test. Compiles iff compose-foundation-layout-wasi is
 * linkable. Exercises the non-@Composable surface — the @Composable
 * WindowInsets.Companion accessors (captionBar/ime/safeDrawing/etc.) need a
 * real composition so they're verified at link-time only.
 */
fun composeFoundationLayoutSmokeTest() {
    val arrCenter = Arrangement.Center
    val arrEnd    = Arrangement.End
    val arrEvenly = Arrangement.SpaceEvenly
    val alignCenter = Alignment.Center

    val pad = PaddingValues(start = 4.dp, top = 8.dp, end = 12.dp, bottom = 16.dp)
    val padAll = PaddingValues(10.dp)
    val padLtr = pad.calculateStartPadding(LayoutDirection.Ltr) // = 4.dp
    val padRtl = pad.calculateStartPadding(LayoutDirection.Rtl) // = 12.dp
    val padEnd = pad.calculateEndPadding(LayoutDirection.Ltr)   // = 12.dp

    val ins1 = WindowInsets(left = 1, top = 2, right = 3, bottom = 4)
    val ins2 = WindowInsets(left = 5, top = 6, right = 7, bottom = 8)
    val insUnion   = ins1.union(ins2)
    val insAdd     = ins1.add(ins2)
    val insExclude = ins1.exclude(ins2)

    WitCanvas.Import.logMessage(
        "foundation-layout smoke: " +
        "arrangements=[${arrCenter}, ${arrEnd}, ${arrEvenly}], align=${alignCenter}, " +
        "pad=${pad}, padAll=${padAll}, padStart-ltr=${padLtr}, padStart-rtl=${padRtl}, padEnd-ltr=${padEnd}, " +
        "ins1=${ins1}, ins2=${ins2}, " +
        "union=${insUnion}, add=${insAdd}, exclude=${insExclude}"
    )
}
