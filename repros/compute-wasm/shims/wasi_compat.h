// WASI compat: BSD `uint` alias wasi-libc lacks. No libc include (avoids a Clang
// module cycle under cxx-interop).
#pragma once
typedef unsigned int uint;
