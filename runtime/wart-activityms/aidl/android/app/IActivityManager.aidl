// Minimal android.app.IActivityManager — ONLY the methods the native C++ client
// (frameworks/native libs/binder/IActivityManager.cpp, used by audioserver/
// cameraserver) actually calls, declared IN THE SAME ORDER as that client's
// transaction-code enum. AIDL assigns codes = FIRST_CALL_TRANSACTION + declaration
// index, so this order makes our generated server's codes match the on-wire codes
// the native client sends. Descriptor = "android.app.IActivityManager" (from the
// package), matching the client's writeInterfaceToken.
//
// `IBinder` is used wherever the real interface takes `IUidObserver` — identical
// wire (writeStrongBinder) — so we don't have to pull in IUidObserver + its deps.
// This is a STUB: a permissive no-op so the callers finish init (see src/main.rs).
package android.app;

interface IActivityManager {
    int openContentUri(String stringUri);                                                       // 0
    void registerUidObserver(IBinder observer, int event, int cutpoint, String callingPackage); // 1
    void unregisterUidObserver(IBinder observer);                                                // 2
    IBinder registerUidObserverForUids(IBinder observer, int event, int cutpoint,
                                       String callingPackage, in int[] uids);                    // 3
    void addUidToObserver(IBinder observerToken, String callingPackage, int uid);                // 4
    void removeUidFromObserver(IBinder observerToken, String callingPackage, int uid);           // 5
    boolean isUidActive(int uid, String callingPackage);                                         // 6
    int getUidProcessState(int uid, String callingPackage);                                      // 7
    int checkPermission(String permission, int pid, int uid);                                    // 8
    oneway void logFgsApiBegin(int apiType, int appUid, int appPid);                             // 9
    oneway void logFgsApiEnd(int apiType, int appUid, int appPid);                               // 10
    oneway void logFgsApiStateChanged(int apiType, int state, int appUid, int appPid);           // 11
}
