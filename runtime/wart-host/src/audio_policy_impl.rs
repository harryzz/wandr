//! Call-audio control via `media.audio_policy` (`IAudioPolicyService`).
//!
//! AAudioService brokers routing for normal in/out, but VoIP / telephony call
//! audio needs two extra knobs AAudio doesn't expose:
//!   - `setPhoneState(IN_COMMUNICATION)` — switch the platform into comms mode
//!     (routing + AEC tuning) for the duration of a call.
//!   - `setForceUse(COMMUNICATION, SPEAKER|NONE|BT_SCO)` — the speaker / earpiece
//!     / Bluetooth toggle.
//!
//! Both take plain int-backed enums, so we bind a *positional stub* of the
//! interface (only 4 of its 107 methods kept real — see
//! `vendor/aidl-stubs/android/media/IAudioPolicyService.aidl`). These calls are
//! normally gated by MODIFY_AUDIO_ROUTING / MODIFY_PHONE_STATE + a privileged
//! SELinux domain; this module is the de-risk probe answering "does a root/su
//! wart caller reach them?" before any call feature is built.

#[cfg(target_os = "android")]
mod binder_path {
    use crate::binder_aidl::android::media::{
        AudioPolicyForceUse::AudioPolicyForceUse,
        AudioPolicyForcedConfig::AudioPolicyForcedConfig,
        IAudioPolicyService::IAudioPolicyService,
    };
    use crate::binder_aidl::android::media::audio::common::AudioMode::AudioMode;

    fn service() -> Option<rsbinder::Strong<dyn IAudioPolicyService>> {
        match rsbinder::hub::get_interface::<dyn IAudioPolicyService>("media.audio_policy") {
            Ok(s)  => { log::info!("audio-policy: media.audio_policy ready"); Some(s) }
            Err(e) => { log::warn!("audio-policy: media.audio_policy unavailable: {e:?}"); None }
        }
    }

    fn mode_name(m: AudioMode) -> &'static str {
        match m {
            AudioMode::NORMAL           => "NORMAL",
            AudioMode::RINGTONE         => "RINGTONE",
            AudioMode::IN_CALL          => "IN_CALL",
            AudioMode::IN_COMMUNICATION => "IN_COMMUNICATION",
            AudioMode::CALL_SCREEN      => "CALL_SCREEN",
            _                           => "other",
        }
    }
    fn cfg_name(c: AudioPolicyForcedConfig) -> &'static str {
        match c {
            AudioPolicyForcedConfig::NONE    => "NONE(earpiece/default)",
            AudioPolicyForcedConfig::SPEAKER => "SPEAKER",
            AudioPolicyForcedConfig::BT_SCO  => "BT_SCO",
            _                                => "other",
        }
    }

    /// Read-only probe: does a root/su caller reach the policy service, and what
    /// are the current phone state + communication routing? No side effects.
    pub fn probe() {
        if let Err(e) = crate::binder::init() {
            log::warn!("audio-policy probe: binder init failed: {e}");
            return;
        }
        let Some(svc) = service() else { return };

        match svc.r#getPhoneState() {
            Ok(m)  => log::info!("audio-policy: getPhoneState = {} ({})", m.0, mode_name(m)),
            Err(e) => log::warn!("audio-policy: getPhoneState DENIED/err: {e:?}"),
        }
        match svc.r#getForceUse(AudioPolicyForceUse::COMMUNICATION) {
            Ok(c)  => log::info!(
                "audio-policy: getForceUse(COMMUNICATION) = {} ({}) — READ ACCESS OK",
                c.0, cfg_name(c),
            ),
            Err(e) => log::warn!("audio-policy: getForceUse DENIED/err: {e:?}"),
        }
    }

    /// Write probe (the speaker/earpiece toggle): read the current COMMUNICATION
    /// routing, force it to `speaker` (or NONE/earpiece), confirm via read-back,
    /// then RESTORE the previous value. Proves we can drive routing without
    /// leaving the device reconfigured. Still a global change for the brief
    /// window, so it's behind its own explicit flag.
    pub fn probe_route(speaker: bool) {
        if let Err(e) = crate::binder::init() {
            log::warn!("audio-policy route: binder init failed: {e}");
            return;
        }
        let Some(svc) = service() else { return };

        let prev = match svc.r#getForceUse(AudioPolicyForceUse::COMMUNICATION) {
            Ok(c)  => { log::info!("audio-policy route: prev = {} ({})", c.0, cfg_name(c)); c }
            Err(e) => { log::warn!("audio-policy route: getForceUse DENIED: {e:?}"); return; }
        };
        let want = if speaker { AudioPolicyForcedConfig::SPEAKER } else { AudioPolicyForcedConfig::NONE };
        match svc.r#setForceUse(AudioPolicyForceUse::COMMUNICATION, want) {
            Ok(())  => log::info!("audio-policy route: setForceUse(COMMUNICATION, {}) OK — WRITE ACCESS GRANTED", cfg_name(want)),
            Err(e)  => { log::warn!("audio-policy route: setForceUse DENIED/err: {e:?} — perm/SELinux?"); return; }
        }
        match svc.r#getForceUse(AudioPolicyForceUse::COMMUNICATION) {
            Ok(c)  => log::info!("audio-policy route: confirmed = {} ({})", c.0, cfg_name(c)),
            Err(e) => log::warn!("audio-policy route: confirm read err: {e:?}"),
        }
        // Restore the previous routing so the device is left as we found it.
        match svc.r#setForceUse(AudioPolicyForceUse::COMMUNICATION, prev) {
            Ok(())  => log::info!("audio-policy route: restored to {} ({})", prev.0, cfg_name(prev)),
            Err(e)  => log::warn!("audio-policy route: RESTORE FAILED: {e:?} — device left in {}", cfg_name(want)),
        }
    }
}

/// Read-only call-audio reachability probe (`--probe-audio-policy`).
#[cfg(target_os = "android")]
pub fn probe() { binder_path::probe(); }
#[cfg(not(target_os = "android"))]
pub fn probe() { log::warn!("audio-policy probe: android-only build"); }

/// Routing write probe (`--probe-audio-policy-route <speaker|earpiece>`):
/// drives the COMMUNICATION force-use then restores it.
#[cfg(target_os = "android")]
pub fn probe_route(speaker: bool) { binder_path::probe_route(speaker); }
#[cfg(not(target_os = "android"))]
pub fn probe_route(_speaker: bool) { log::warn!("audio-policy route: android-only build"); }
