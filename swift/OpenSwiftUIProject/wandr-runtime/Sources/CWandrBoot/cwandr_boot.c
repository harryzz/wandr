#include "cwandr_boot.h"

// Defined by the app's `@main struct App` (Swift emits it for -mexec-model=reactor); resolved at the
// final wasm link. Referencing it here also keeps it alive (it would otherwise be dead-stripped).
extern int __main_argc_argv(int argc, char **argv);

int wandr_run_app_main(void) {
    return __main_argc_argv(0, 0);
}
