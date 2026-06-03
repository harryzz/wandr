// STUB of android.media.IAudioPolicyService for the wart host (call-audio probe).
//
// Transaction codes in binder are POSITIONAL (FIRST_CALL_TRANSACTION + the
// method's 0-based declaration index), so to call a real method we must keep
// EVERY method at its true position. We keep 5 real — setPhoneState (4),
// setForceUse (5), getForceUse (6), getDevicesForAttributes (25, task 76),
// getPhoneState (55) — so every other slot
// is a `void slot_N()` placeholder that preserves the index without pulling in
// the ~100 transitive parcelables the real interface references. Same pattern
// as the IInputMethodManager stub (build.rs). The 3 enums used by the kept
// methods are simple @Backing(int) types copied alongside this file.
//
// Method order/count mirror android-15.0.0_r36 libaudioclient
// IAudioPolicyService.aidl (106 methods, 0..105) — derived by stripping
// comments and splitting the interface body on method terminators. If that
// interface changes upstream, regenerate this stub.
package android.media;

import android.media.AudioPolicyForceUse;
import android.media.AudioPolicyForcedConfig;
import android.media.audio.common.AudioMode;
import android.media.audio.common.AudioAttributes;
import android.media.audio.common.AudioDevice;
import android.media.audio.common.AudioDeviceDescription;
import android.media.audio.common.Int;
import android.media.AudioPortFw;
import android.media.AudioPortRole;
import android.media.AudioPortType;

interface IAudioPolicyService {
    void slot_0();
    void slot_1();
    void slot_2();
    void slot_3();
    void setPhoneState(AudioMode state, int uid);
    void setForceUse(AudioPolicyForceUse usage, AudioPolicyForcedConfig config);
    AudioPolicyForcedConfig getForceUse(AudioPolicyForceUse usage);
    void slot_7();
    void slot_8();
    void slot_9();
    void slot_10();
    void slot_11();
    void slot_12();
    void slot_13();
    void slot_14();
    void slot_15();
    void slot_16();
    void slot_17();
    void slot_18();
    void slot_19();
    // indices 20-23 — task 76 volume control (P8). The attributes-based volume
    // API: get/set an index for an AudioAttributes on a device, plus the
    // device-independent max/min range. Verified indices (parsed from the
    // vendored real AIDL; anchors setPhoneState=4 / getDevicesForAttributes=25 /
    // getPhoneState=55 all match the device).
    void setVolumeIndexForAttributes(in AudioAttributes attr, in AudioDeviceDescription device, int index, boolean muted);
    int getVolumeIndexForAttributes(in AudioAttributes attr, in AudioDeviceDescription device);
    int getMaxVolumeIndexForAttributes(in AudioAttributes attr);
    int getMinVolumeIndexForAttributes(in AudioAttributes attr);
    void slot_24();
    // index 25 — task 76: the policy's own "where would this route now" answer.
    // Returns the common AudioDevice[]; AudioDeviceAddress is a union (rsbinder-
    // aidl 0.8.0 supports unions). Wire layout may differ from the device's
    // framework AudioAttributes — treat the result as binder-reachability
    // evidence; dumpsys media.audio_policy is the authoritative routing source.
    AudioDevice[] getDevicesForAttributes(in AudioAttributes attr, boolean forVolume);
    void slot_26();
    void slot_27();
    void slot_28();
    void slot_29();
    void slot_30();
    void slot_31();
    void slot_32();
    void slot_33();
    void slot_34();
    void slot_35();
    void slot_36();
    void slot_37();
    void slot_38();
    void slot_39();
    void slot_40();
    void slot_41();
    void slot_42();
    // index 43 — task 76 #6: enumerate audio ports over binder (native
    // audioserver) instead of shelling dumpsys. Returns the framework AudioPortFw
    // (deep, 6 unions, all vendored) — trialling whether rsbinder-aidl 0.8.0
    // generates + decodes it.
    int listAudioPorts(AudioPortRole role, AudioPortType type, inout Int count, out AudioPortFw[] ports);
    void slot_44();
    void slot_45();
    void slot_46();
    void slot_47();
    void slot_48();
    void slot_49();
    void slot_50();
    void slot_51();
    void slot_52();
    void slot_53();
    void slot_54();
    AudioMode getPhoneState();
    void slot_56();
    void slot_57();
    void slot_58();
    void slot_59();
    void slot_60();
    void slot_61();
    void slot_62();
    void slot_63();
    void slot_64();
    void slot_65();
    void slot_66();
    void slot_67();
    void slot_68();
    void slot_69();
    void slot_70();
    void slot_71();
    void slot_72();
    void slot_73();
    void slot_74();
    void slot_75();
    void slot_76();
    void slot_77();
    void slot_78();
    void slot_79();
    void slot_80();
    void slot_81();
    void slot_82();
    void slot_83();
    void slot_84();
    void slot_85();
    void slot_86();
    void slot_87();
    void slot_88();
    void slot_89();
    void slot_90();
    void slot_91();
    void slot_92();
    void slot_93();
    void slot_94();
    void slot_95();
    void slot_96();
    void slot_97();
    void slot_98();
    void slot_99();
    void slot_100();
    void slot_101();
    void slot_102();
    void slot_103();
    void slot_104();
    void slot_105();
}
