// proxlog — minimal NDK ASensorManager proximity logger, for the ART vs --no-art
// delivery A/B. Uses the LOW-LEVEL sensor path (ASensorManager → ISensorServer →
// sensorservice), the same path the Java SystemSensorManager uses — NOT the
// high-level frameworks.sensorservice.ISensorManager that wart-sensormanager/the
// arbiter use. Logs each proximity event's arrival wall-clock so it can be compared
// against the SLPI's ASH "PRX_STATE" logcat timestamp to get end-to-end delivery
// latency. Build with the NDK for aarch64-android, run as root in both modes.
#include <android/looper.h>
#include <android/sensor.h>
#include <stdio.h>
#include <time.h>

static double now_ms(void) {
    struct timespec t;
    clock_gettime(CLOCK_REALTIME, &t);
    return t.tv_sec * 1000.0 + t.tv_nsec / 1e6;
}

#define STEP(s) do { printf("PROXLOG-STEP %s t=%.3f\n", s, now_ms()); fflush(stdout); } while (0)

int main(void) {
    STEP("getInstance...");
    ASensorManager* mgr = ASensorManager_getInstance();
    STEP("getInstance-done");
    if (!mgr) { printf("PROXLOG: no ASensorManager\n"); fflush(stdout); return 1; }

    STEP("getDefaultSensor...");
    const ASensor* prox = ASensorManager_getDefaultSensor(mgr, ASENSOR_TYPE_PROXIMITY);
    STEP("getDefaultSensor-done");
    if (!prox) { printf("PROXLOG: no proximity sensor\n"); fflush(stdout); return 1; }

    ALooper* looper = ALooper_prepare(ALOOPER_PREPARE_ALLOW_NON_CALLBACKS);
    STEP("createEventQueue...");
    ASensorEventQueue* q = ASensorManager_createEventQueue(mgr, looper, 3, NULL, NULL);
    STEP("createEventQueue-done");
    if (!q) { printf("PROXLOG: createEventQueue failed\n"); fflush(stdout); return 1; }

    ASensorEventQueue_enableSensor(q, prox);
    ASensorEventQueue_setEventRate(q, prox, 50000); // 50 ms, same as the arbiter
    printf("PROXLOG: up (proximity, 50ms) t=%.3f — cover/uncover now\n", now_ms());
    fflush(stdout);

    for (;;) {
        ALooper_pollOnce(-1, NULL, NULL, NULL);
        ASensorEvent e;
        while (ASensorEventQueue_getEvents(q, &e, 1) > 0) {
            printf("PROXLOG t=%.3f distance=%.2f\n", now_ms(), e.distance);
            fflush(stdout);
        }
    }
    return 0;
}
