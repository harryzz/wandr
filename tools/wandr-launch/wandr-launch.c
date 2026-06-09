// wandr-launch — run a target in (roughly) system_server's security context, so a
// wandr native process can use the surviving native daemons (SurfaceFlinger, …) with
// the Java framework stopped (task 83 / ART-off).
//
// The dev stand-in for what `init.rc` does natively in a flashable image
// (user/group/capabilities/seclabel). Launched as root (via `su -c`), it drops to
// uid=system + gid=system,input(,graphics) and retains CAP_BLOCK_SUSPEND (+ a couple
// input-relevant caps) across the exec, because:
//   - SurfaceFlinger short-circuits its ACCESS_SURFACE_FLINGER permission check for
//     uid system/graphics; bare root HANGS (the permission service is in the dead
//     system_server).
//   - EventHub aborts without CAP_BLOCK_SUSPEND (EPOLLWAKEUP).
//   - /dev/input is gid `input` (1004).
// SELinux is handled separately in dev (`setenforce 0`); a real image ships a wandr
// sepolicy domain instead.
//
// Build (NDK, plain libc — no AOSP internal headers):
//   $CC_aarch64_linux_android -O2 -o wandr-launch wandr-launch.c
//
// Usage: wandr-launch <program> [args...]

#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <grp.h>
#include <string.h>
#include <errno.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <linux/capability.h>

#ifndef PR_CAP_AMBIENT
#define PR_CAP_AMBIENT 47
#define PR_CAP_AMBIENT_RAISE 2
#endif

#define AID_SYSTEM   1000
#define AID_GRAPHICS 1003
#define AID_INPUT    1004

#ifndef CAP_SYS_NICE
#define CAP_SYS_NICE 23
#endif
#ifndef CAP_WAKE_ALARM
#define CAP_WAKE_ALARM 35
#endif
#ifndef CAP_BLOCK_SUSPEND
#define CAP_BLOCK_SUSPEND 36
#endif

// Grant exactly the caps an ART-off native service needs:
//   CAP_BLOCK_SUSPEND — EventHub EPOLLWAKEUP (hard requirement)
//   CAP_SYS_NICE      — InputReader/Dispatcher set thread priority/affinity
//   CAP_WAKE_ALARM    — timerfd/alarm wakeups
static int set_caps(void) {
    struct __user_cap_header_struct hdr;
    struct __user_cap_data_struct data[2];
    memset(&hdr, 0, sizeof(hdr));
    memset(data, 0, sizeof(data));
    hdr.version = _LINUX_CAPABILITY_VERSION_3;
    hdr.pid = 0; // self

    unsigned long long caps =
        (1ULL << CAP_BLOCK_SUSPEND) | (1ULL << CAP_SYS_NICE) | (1ULL << CAP_WAKE_ALARM);
    data[0].effective = data[0].permitted = data[0].inheritable = (__u32)(caps & 0xffffffffULL);
    data[1].effective = data[1].permitted = data[1].inheritable = (__u32)(caps >> 32);

    if (syscall(SYS_capset, &hdr, data) != 0) {
        fprintf(stderr, "wandr-launch: capset failed: %s\n", strerror(errno));
        return -1;
    }
    return 0;
}

int main(int argc, char** argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: %s <program> [args...]\n", argv[0]);
        return 2;
    }

    // Retain permitted caps across the setuid() drop from root.
    if (prctl(PR_SET_KEEPCAPS, 1, 0, 0, 0) != 0) {
        fprintf(stderr, "wandr-launch: PR_SET_KEEPCAPS failed: %s\n", strerror(errno));
    }

    gid_t groups[] = { AID_SYSTEM, AID_GRAPHICS, AID_INPUT };
    if (setgroups(sizeof(groups) / sizeof(groups[0]), groups) != 0) {
        fprintf(stderr, "wandr-launch: setgroups failed: %s\n", strerror(errno));
    }
    if (setgid(AID_SYSTEM) != 0) {
        fprintf(stderr, "wandr-launch: setgid(system) failed: %s\n", strerror(errno));
    }
    if (setuid(AID_SYSTEM) != 0) {
        fprintf(stderr, "wandr-launch: setuid(system) failed: %s\n", strerror(errno));
        return 1;
    }

    set_caps();

    // Make the caps survive execve into a non-privileged binary (ambient set;
    // requires each cap be in permitted+inheritable, set above).
    prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE, CAP_BLOCK_SUSPEND, 0, 0);
    prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE, CAP_SYS_NICE, 0, 0);
    prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE, CAP_WAKE_ALARM, 0, 0);

    execvp(argv[1], &argv[1]);
    fprintf(stderr, "wandr-launch: execvp(%s) failed: %s\n", argv[1], strerror(errno));
    return 127;
}
