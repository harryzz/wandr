// Stub for android.content.AttributionSourceState — the real one lives
// in frameworks/base/core/java/android/content/. Referenced by
// aaudio.StreamRequest. We never construct a real one across binder
// (we'll fill in pid/uid manually if needed), so an empty parcelable
// satisfies the import.
package android.content;
parcelable AttributionSourceState;
