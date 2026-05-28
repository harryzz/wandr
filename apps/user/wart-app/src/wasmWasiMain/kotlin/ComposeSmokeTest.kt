package testapp

import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue

// Smoke test — proves the Compose Compiler runs over the test-app's Kotlin
// source set on wasmWasi. The function isn't called yet; the compile alone
// is the test. The plugin would fail FIR→IR if codegen didn't accept wasmWasi.

@Composable
fun ComposeSmokeTest(): Int {
    var counter by remember { mutableStateOf(0) }
    counter += 1
    return counter
}
