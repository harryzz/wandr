cd ~/wandr/tests/OpenSwiftUIProject/oag-baseline
OPENATTRIBUTEGRAPH_USE_LOCAL_DEPS=1 OPENATTRIBUTEGRAPH_OPENATTRIBUTESHIMS_COMPUTE=1 \
swift build --swift-sdk swift-6.3.2-RELEASE_wasm \
  -Xcc -I/home/harry/wandr/tests/OpenSwiftUIProject/wasi-shims -Xcc -include -Xcc /home/harry/wandr/tests/OpenSwiftUIProject/wasi-shims/wasi_compat.h \
  -Xcc -fno-exceptions -Xcc -DSWIFT_INLINE_NAMESPACE=__runtime \
  -Xcc -D_WASI_EMULATED_SIGNAL -Xcc -D_WASI_EMULATED_MMAN -Xcc -D_WASI_EMULATED_PROCESS_CLOCKS
echo "BUILD_EXIT=$?"
