cd ~/wandr/swift/OpenSwiftUIProject/tests/oag-baseline
OPENATTRIBUTEGRAPH_USE_LOCAL_DEPS=1 OPENATTRIBUTEGRAPH_OPENATTRIBUTESHIMS_COMPUTE=1 \
swift build --swift-sdk swift-6.3.2-RELEASE_wasm \
  -Xcc -I/home/harry/wandr/swift/OpenSwiftUIProject/wandr/wasi-shims -Xcc -include -Xcc /home/harry/wandr/swift/OpenSwiftUIProject/wandr/wasi-shims/wasi_compat.h \
  -Xcc -fno-exceptions -Xcc -DSWIFT_INLINE_NAMESPACE=__runtime \
  -Xcc -D_WASI_EMULATED_SIGNAL -Xcc -D_WASI_EMULATED_MMAN -Xcc -D_WASI_EMULATED_PROCESS_CLOCKS \
  -Xlinker -z -Xlinker stack-size=8388608
  # ^ Guest shadow stack = 8 MiB (linear-memory C stack, set at link time). The wasi default
  # (~128 KiB) is far below the host's wasm call-stack budget (wandr-host max_wasm_stack=4 MiB /
  # async_stack_size=8 MiB) and the native thread stack (8 MiB) that the linux build runs on.
  # AttributeGraph recurses on first evaluation (lazy input-edge creation: input_value_ref_slow ->
  # update_attribute -> update -> rule -> input_value_ref_slow ...), so a deep attribute chain
  # overflowed the tiny default shadow stack into the heap and faulted with a wild pointer
  # (oagupdate: depth 50 OK, 200+ OOB). 8 MiB gives guest/host/native parity. (UpdateStack::update
  # was also made iterative so the steady-state re-eval path no longer grows the C++ stack at all.)
echo "BUILD_EXIT=$?"
