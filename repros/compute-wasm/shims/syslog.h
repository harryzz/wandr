// WASI POSIX shim for OpenAttributeGraph's Platform/log (task 114 probe).
// WASI has no syslog daemon; redirect to stderr, no-op the rest. static inline =
// no separate .c to link.
#pragma once
#include <stdio.h>
#include <stdarg.h>
#define LOG_EMERG 0
#define LOG_ALERT 1
#define LOG_CRIT 2
#define LOG_ERR 3
#define LOG_WARNING 4
#define LOG_NOTICE 5
#define LOG_INFO 6
#define LOG_DEBUG 7
#define LOG_PID 0x01
#define LOG_CONS 0x02
#define LOG_ODELAY 0x04
#define LOG_NDELAY 0x08
#define LOG_USER (1<<3)
static inline void openlog(const char *ident, int option, int facility) { (void)ident;(void)option;(void)facility; }
static inline void closelog(void) {}
static inline void setlogmask(int mask) { (void)mask; }
static inline void syslog(int priority, const char *fmt, ...) {
    (void)priority; va_list ap; va_start(ap, fmt); vfprintf(stderr, fmt, ap); va_end(ap); fputc('\n', stderr);
}
