package testapp.compose

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos

// Minimal `animateFloatAsState` with the same signature as the upstream
// `androidx.compose.animation.core` API. The full upstream module pulls in
// `compose.ui.graphics` (Path / Bezier), `compose.ui.platform`, etc. — none
// of which compile on wasmWasi. This re-implementation is ~25 lines and
// supports linear + ease-in-out tweens, which is all the demo needs.
//
// Usage:
//   val width by animateFloatAsState(targetValue = if (open) 320f else 80f)

@Composable
fun animateFloatAsState(
    targetValue: Float,
    durationMillis: Int = 300,
    easing: (Float) -> Float = EaseInOutCubic,
): State<Float> {
    val state = remember { mutableStateOf(targetValue) }
    LaunchedEffect(targetValue, durationMillis) {
        val startValue = state.value
        if (startValue == targetValue) return@LaunchedEffect
        if (durationMillis <= 0) {
            state.value = targetValue
            return@LaunchedEffect
        }
        var startNanos = 0L
        while (true) {
            val finished = withFrameNanos { now ->
                if (startNanos == 0L) startNanos = now
                val elapsedMs = (now - startNanos) / 1_000_000L
                val t = (elapsedMs.toFloat() / durationMillis).coerceIn(0f, 1f)
                state.value = startValue + (targetValue - startValue) * easing(t)
                t >= 1f
            }
            if (finished) break
        }
        state.value = targetValue
    }
    return state
}

/** ease-in-out cubic — matches Compose's default "FastOutSlowIn" feel. */
val EaseInOutCubic: (Float) -> Float = { t ->
    if (t < 0.5f) 4f * t * t * t
    else {
        val u = -2f * t + 2f
        1f - (u * u * u) / 2f
    }
}

/** Linear easing — straight-line interpolation. */
val LinearEasing: (Float) -> Float = { it }
