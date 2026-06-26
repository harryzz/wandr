// Stub for the Apple-only Graph.mm symbol IAG::Graph::print_cycle(data::ptr<Node>), referenced
// from UpdateStack.cpp on the cycle-detection path. Graph.mm is Obj-C++ and isn't compiled on wasi,
// so the symbol is otherwise undefined at link. No-op (cycle printing is a debug-only diagnostic).
__attribute__((used)) void _ZN3IAG5Graph11print_cycleENS_4data3ptrINS_4NodeEEE(void) {}
