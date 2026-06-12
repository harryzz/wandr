package testapp

import androidx.compose.animation.core.AnimationVector1D
import androidx.compose.animation.core.AnimationVector2D
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.Easing
import androidx.compose.animation.core.EaseInOut
import androidx.compose.animation.core.EaseOutBack
import androidx.compose.animation.core.FastOutLinearInEasing
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.FloatExponentialDecaySpec
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.LinearOutSlowInEasing
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.VectorConverter
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.unit.dp

/**
 * compose-animation-core smoke test. Compiles iff compose-animation-core-wasi
 * is linkable. Exercises the non-@Composable surface: Easing fns,
 * AnimationVectors, AnimationSpecs (tween/spring), VectorConverter.
 */
fun composeAnimationCoreSmokeTest() {
    // Built-in easing curves
    val linAt33   = LinearEasing.transform(0.33f)
    val fastSlow  = FastOutSlowInEasing.transform(0.5f)
    val easeInOut = EaseInOut.transform(0.5f)
    val easeBack  = EaseOutBack.transform(0.5f)  // overshoots > 1.0

    // Custom cubic bezier easing
    val customBez = CubicBezierEasing(0.4f, 0.0f, 0.2f, 1.0f).transform(0.5f)

    // Animation vectors
    val v1 = AnimationVector1D(42f)
    val v2 = AnimationVector2D(1f, 2f)

    // Animation specs (non-@Composable factories)
    val tween100 = tween<Float>(durationMillis = 100, delayMillis = 50)
    val springLow = spring<Float>(dampingRatio = Spring.DampingRatioLowBouncy, stiffness = Spring.StiffnessLow)
    val decay = FloatExponentialDecaySpec(frictionMultiplier = 1.5f)

    // VectorConverter — Offset <-> AnimationVector2D
    val offsetCnv = Offset.VectorConverter
    val vecFromOffset = offsetCnv.convertToVector(Offset(10f, 20f))
    val offsetFromVec = offsetCnv.convertFromVector(AnimationVector2D(7f, 8f))

    // dp converter
    val dpCnv = androidx.compose.ui.unit.Dp.VectorConverter
    val vecFromDp = dpCnv.convertToVector(24.dp)

    logMessage(
        "compose-animation-core smoke: " +
        "easings={linear=${linAt33}, fastSlow=${fastSlow}, inOut=${easeInOut}, outBack=${easeBack}, customBez=${customBez}}, " +
        "vectors=[${v1}, ${v2}], " +
        "specs={tween=${tween100::class.simpleName}, spring=${springLow::class.simpleName}, decay=${decay::class.simpleName}}, " +
        "offset-cnv: Offset(10,20)→${vecFromOffset}, Vector(7,8)→${offsetFromVec}, " +
        "24dp→${vecFromDp}"
    )
}
