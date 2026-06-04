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
    use crate::binder_aidl::android::media::{
        AudioPortFw::AudioPortFw,
        AudioPortRole::AudioPortRole,
        AudioPortType::AudioPortType,
    };
    use crate::binder_aidl::android::media::audio::common::Int::Int;
    use crate::binder_aidl::android::media::audio::common::AudioPortExt::AudioPortExt;
    use crate::binder_aidl::android::media::audio::common::{
        AudioAttributes::AudioAttributes,
        AudioContentType::AudioContentType,
        AudioDeviceDescription::AudioDeviceDescription,
        AudioDeviceType::AudioDeviceType,
        AudioSource::AudioSource,
        AudioUsage::AudioUsage,
    };

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

    // The originator uid for setPhoneState (the mode owner the policy service
    // tracks). We run as root; report our real uid.
    extern "C" { fn getuid() -> u32; }

    /// wart-arbiter-audio M3 — set the global audio mode (the call-owner host
    /// applies this when the arbiter starts/ends a comms session). `comm=true` →
    /// IN_COMMUNICATION (VoIP routing + AEC tuning); `false` → NORMAL.
    pub fn set_mode(comm: bool) {
        let Some(svc) = service() else { return };
        let state = if comm { AudioMode::IN_COMMUNICATION } else { AudioMode::NORMAL };
        let uid = unsafe { getuid() } as i32;
        match svc.r#setPhoneState(state, uid) {
            Ok(())  => log::info!("audio-policy: setPhoneState {} (uid={uid})", mode_name(state)),
            Err(e)  => log::warn!("audio-policy: setPhoneState {} failed: {e:?}", mode_name(state)),
        }
    }

    /// wart-arbiter-audio M3 — set the communication routing (the speaker /
    /// earpiece toggle). `speaker=true` → SPEAKER; `false` → NONE (earpiece).
    pub fn set_route(speaker: bool) {
        let Some(svc) = service() else { return };
        let cfg = if speaker { AudioPolicyForcedConfig::SPEAKER } else { AudioPolicyForcedConfig::NONE };
        match svc.r#setForceUse(AudioPolicyForceUse::COMMUNICATION, cfg) {
            Ok(())  => log::info!("audio-policy: setForceUse COMMUNICATION {}", cfg_name(cfg)),
            Err(e)  => log::warn!("audio-policy: setForceUse {} failed: {e:?}", cfg_name(cfg)),
        }
    }

    /// Task-76 read-only probe: ask the policy service where each usage would
    /// route RIGHT NOW (`getDevicesForAttributes`, transaction index 25). This
    /// is the binder equivalent of the routing the refactor will consult at
    /// runtime instead of shelling `dumpsys`. The `AudioAttributes` wire layout
    /// is the common (HAL) shape; if it differs from the device's framework
    /// shape the call returns an error/garbage — either way the log is
    /// API-of-record evidence ("does AudioDevice[] decode over binder?").
    /// `dumpsys media.audio_policy` strategy→device lines remain authoritative.
    pub fn probe_devices_for_attributes() {
        if let Err(e) = crate::binder::init() {
            log::warn!("audio-caps devices-for-attr: binder init failed: {e}");
            return;
        }
        let Some(svc) = service() else { return };
        let usages: &[(&str, AudioUsage, AudioContentType, AudioSource)] = &[
            ("MEDIA",               AudioUsage::MEDIA,               AudioContentType::MUSIC,   AudioSource::DEFAULT),
            ("VOICE_COMMUNICATION", AudioUsage::VOICE_COMMUNICATION, AudioContentType::SPEECH,  AudioSource::DEFAULT),
            ("NOTIFICATION",        AudioUsage::NOTIFICATION,        AudioContentType::SONIFICATION, AudioSource::DEFAULT),
            ("ALARM",               AudioUsage::ALARM,               AudioContentType::SONIFICATION, AudioSource::DEFAULT),
        ];
        for (label, usage, content, source) in usages {
            let attr = AudioAttributes {
                r#contentType: *content,
                r#usage:       *usage,
                r#source:      *source,
                r#flags:       0,
                r#tags:        Vec::new(),
                ..Default::default()
            };
            match svc.r#getDevicesForAttributes(&attr, false) {
                Ok(devs) => log::info!(
                    "audio-caps: getDevicesForAttributes({label}) -> {} device(s): {:?}",
                    devs.len(), devs,
                ),
                Err(e) => log::warn!(
                    "audio-caps: getDevicesForAttributes({label}) binder err / decode gap: {e:?}",
                ),
            }
        }
    }

    // ── Volume (task 76 P8) ──────────────────────────────────────────────────
    // The attributes-based volume API on the policy service (verified indices
    // 20-23). Volume is stored per (attributes/stream, device); the index runs
    // over a device-independent [min,max] range (media = 0..25 on this device).
    // The arbiter decides the policy (target stream, level); these are the host
    // appliers + read accessors.

    fn media_attr() -> AudioAttributes {
        AudioAttributes {
            r#contentType: AudioContentType::MUSIC,
            r#usage:       AudioUsage::MEDIA,
            r#source:      AudioSource::DEFAULT,
            r#flags:       0,
            r#tags:        Vec::new(),
            ..Default::default()
        }
    }
    fn dev_desc(t: AudioDeviceType) -> AudioDeviceDescription {
        AudioDeviceDescription { r#type: t, r#connection: String::new() }
    }

    /// Media volume range `[min, max]` (device-independent). `None` if the
    /// service is unreachable.
    pub fn media_volume_range() -> Option<(i32, i32)> {
        let svc = service()?;
        let attr = media_attr();
        let max = svc.r#getMaxVolumeIndexForAttributes(&attr).ok()?;
        let min = svc.r#getMinVolumeIndexForAttributes(&attr).ok()?;
        Some((min, max))
    }
    /// Current media volume index on `device` (e.g. `OUT_SPEAKER`).
    pub fn get_media_volume(device: AudioDeviceType) -> Option<i32> {
        let svc = service()?;
        svc.r#getVolumeIndexForAttributes(&media_attr(), &dev_desc(device)).ok()
    }
    /// Set the media volume index on `device`, clamped to `[min, max]`. Returns
    /// the index actually applied (post-clamp), or `None` on failure.
    pub fn set_media_volume(device: AudioDeviceType, index: i32) -> Option<i32> {
        let svc = service()?;
        let (min, max) = media_volume_range().unwrap_or((0, index.max(0)));
        let idx = index.clamp(min, max);
        match svc.r#setVolumeIndexForAttributes(&media_attr(), &dev_desc(device), idx, false) {
            Ok(())  => { log::info!("audio-policy: media volume {idx} on {device:?} [{min}..{max}]"); Some(idx) }
            Err(e)  => { log::warn!("audio-policy: setVolumeIndexForAttributes err: {e:?}"); None }
        }
    }

    /// Apply a one-step MEDIA volume change on `device` (the **arbiter** picks
    /// which device — speaker or earpiece — and which host applies; this is the
    /// pure applier). `speaker` selects OUT_SPEAKER vs OUT_SPEAKER_EARPIECE. Our
    /// call audio rides the MEDIA stream (USAGE_MEDIA), so MEDIA volume is the
    /// lever for both call and media. Step ≈ 1/10 of the range (≥1).
    pub fn adjust_volume_on(speaker: bool, up: bool) {
        let device = if speaker { AudioDeviceType::OUT_SPEAKER } else { AudioDeviceType::OUT_SPEAKER_EARPIECE };
        let (min, max) = media_volume_range().unwrap_or((0, 15));
        let step = ((max - min) / 10).max(1);
        let Some(cur) = get_media_volume(device) else {
            log::warn!("audio-policy: volume — read failed");
            return;
        };
        let next = if up { cur + step } else { cur - step };
        set_media_volume(device, next);
    }

    /// Apply output mute/unmute on `device` (arbiter-decided). Uses the policy
    /// volume setter's `muted` flag, preserving the current index so unmute
    /// restores the prior level. `speaker` selects OUT_SPEAKER vs earpiece.
    pub fn set_media_mute(speaker: bool, muted: bool) {
        let Some(svc) = service() else { return };
        let device = if speaker { AudioDeviceType::OUT_SPEAKER } else { AudioDeviceType::OUT_SPEAKER_EARPIECE };
        let attr = media_attr();
        let dev = dev_desc(device);
        let cur = svc.r#getVolumeIndexForAttributes(&attr, &dev).unwrap_or(0);
        match svc.r#setVolumeIndexForAttributes(&attr, &dev, cur, muted) {
            Ok(())  => log::info!("audio-policy: media {} on {device:?} (idx={cur})", if muted { "MUTED" } else { "unmuted" }),
            Err(e)  => log::warn!("audio-policy: setVolumeIndexForAttributes(mute) err: {e:?}"),
        }
    }

    /// Read-only-ish volume probe (`--probe-audio-volume`): reads the media
    /// range + current index on speaker & earpiece, then sets the speaker index
    /// to max, reads it back, and restores the previous value (self-restoring,
    /// like `probe_route`). Proves the write path before keys/arbiter wire it.
    pub fn probe_volume() {
        if let Err(e) = crate::binder::init() {
            log::warn!("audio-caps volume: binder init failed: {e}");
            return;
        }
        let Some(svc) = service() else { return };
        let attr = media_attr();
        log::info!("audio-caps: media volume range min={:?} max={:?}",
            svc.r#getMinVolumeIndexForAttributes(&attr),
            svc.r#getMaxVolumeIndexForAttributes(&attr));
        for (label, t) in [("speaker", AudioDeviceType::OUT_SPEAKER),
                           ("earpiece", AudioDeviceType::OUT_SPEAKER_EARPIECE)] {
            match svc.r#getVolumeIndexForAttributes(&attr, &dev_desc(t)) {
                Ok(v)  => log::info!("audio-caps: media volume on {label} = {v}"),
                Err(e) => log::warn!("audio-caps: getVolumeIndexForAttributes({label}) err: {e:?}"),
            }
        }
        let dev = dev_desc(AudioDeviceType::OUT_SPEAKER);
        let prev = match svc.r#getVolumeIndexForAttributes(&attr, &dev) {
            Ok(v)  => v,
            Err(e) => { log::warn!("audio-caps: volume read err: {e:?}"); return; }
        };
        let max = svc.r#getMaxVolumeIndexForAttributes(&attr).unwrap_or(prev);
        match svc.r#setVolumeIndexForAttributes(&attr, &dev, max, false) {
            Ok(())  => log::info!("audio-caps: set speaker media volume {prev} -> {max} — WRITE ACCESS OK"),
            Err(e)  => { log::warn!("audio-caps: set volume DENIED/err: {e:?} — perm/SELinux?"); return; }
        }
        if let Ok(v) = svc.r#getVolumeIndexForAttributes(&attr, &dev) {
            log::info!("audio-caps: confirmed speaker media volume = {v}");
        }
        match svc.r#setVolumeIndexForAttributes(&attr, &dev, prev, false) {
            Ok(())  => log::info!("audio-caps: restored speaker media volume to {prev}"),
            Err(e)  => log::warn!("audio-caps: RESTORE FAILED: {e:?}"),
        }
    }

    /// Map the common `AudioDeviceType` to a legacy `AUDIO_DEVICE_(OUT|IN)_*`
    /// token — the same shape `dumpsys` produced — so the routing core's
    /// type-token lookup works unchanged. Speaker/earpiece are mapped precisely
    /// (routing needs them); others descriptively.
    fn device_type_token(t: AudioDeviceType) -> String {
        if t == AudioDeviceType::OUT_SPEAKER          { "AUDIO_DEVICE_OUT_SPEAKER".into() }
        else if t == AudioDeviceType::OUT_SPEAKER_EARPIECE { "AUDIO_DEVICE_OUT_EARPIECE".into() }
        else if t == AudioDeviceType::OUT_SPEAKER_SAFE     { "AUDIO_DEVICE_OUT_SPEAKER_SAFE".into() }
        else if t == AudioDeviceType::OUT_TELEPHONY_TX     { "AUDIO_DEVICE_OUT_TELEPHONY_TX".into() }
        else { format!("AUDIO_DEVICE_TYPE_{}", t.0) }
    }

    /// Task 76 #6 — enumerate audio **device** ports over binder (native
    /// audioserver, `listAudioPorts`) instead of parsing `dumpsys`. Returns the
    /// device-independent port table the routing core consumes (port id + type +
    /// direction). Requires rsbinder ≥ master/0.9.0 (0.8.0 mis-decoded
    /// `AudioPortFw`). Empty on error. Two-pass: count, then fetch (the ports
    /// vec stays empty — the service allocates + fills it).
    pub fn enumerate_device_ports() -> Vec<crate::audio_routing::AudioDeviceCaps> {
        use crate::audio_routing::{AudioDeviceCaps, Direction};
        let mut out = Vec::new();
        if crate::binder::init().is_err() { return out; }
        let Some(svc) = service() else { return out };
        let mut count = Int { r#value: 0 };
        let mut tmp: Vec<AudioPortFw> = Vec::new();
        if svc.r#listAudioPorts(AudioPortRole::NONE, AudioPortType::DEVICE, &mut count, &mut tmp).is_err() {
            log::warn!("audio-routing: listAudioPorts(count) failed"); return out;
        }
        let mut count2 = Int { r#value: count.r#value.max(0) };
        let mut ports: Vec<AudioPortFw> = Vec::new();
        if let Err(e) = svc.r#listAudioPorts(AudioPortRole::NONE, AudioPortType::DEVICE, &mut count2, &mut ports) {
            log::warn!("audio-routing: listAudioPorts(fetch) failed: {e:?}"); return out;
        }
        for p in &ports {
            // A device port's ext carries the AudioDevice (type + address).
            let dev_type = match &p.r#hal.r#ext {
                AudioPortExt::r#Device(d) => d.r#device.r#type.r#type,
                _ => continue,
            };
            let direction = if dev_type.0 >= AudioDeviceType::OUT_DEFAULT.0 {
                Direction::Output
            } else {
                Direction::Input
            };
            out.push(AudioDeviceCaps {
                direction,
                port_id: p.r#hal.r#id,
                name: p.r#hal.r#name.clone(),
                type_token: device_type_token(dev_type),
                formats: Vec::new(),
                sample_rates: Vec::new(),
                channel_masks: Vec::new(),
            });
        }
        out
    }

    /// `--probe-audio-ports`: log the binder-enumerated device ports.
    pub fn probe_list_audio_ports() {
        let ports = enumerate_device_ports();
        log::info!("audio-caps: listAudioPorts (binder) -> {} device ports", ports.len());
        for d in &ports {
            log::info!("audio-caps: port id={} {:?} {} name={:?}",
                d.port_id, d.direction, d.type_token, d.name);
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

/// Task-76 read-only routing probe (`getDevicesForAttributes` per usage).
#[cfg(target_os = "android")]
pub fn probe_devices_for_attributes() { binder_path::probe_devices_for_attributes(); }
#[cfg(not(target_os = "android"))]
pub fn probe_devices_for_attributes() { log::warn!("audio-caps devices-for-attr: android-only build"); }

/// Task-76 P8 volume probe (`--probe-audio-volume`): read range + speaker/
/// earpiece media volume, set speaker to max, read back, restore.
#[cfg(target_os = "android")]
pub fn probe_volume() { binder_path::probe_volume(); }
#[cfg(not(target_os = "android"))]
pub fn probe_volume() { log::warn!("audio-caps volume: android-only build"); }

/// Task-76 #6 port-enum probe (`--probe-audio-ports`): listAudioPorts over binder.
#[cfg(target_os = "android")]
pub fn probe_list_audio_ports() { binder_path::probe_list_audio_ports(); }
#[cfg(not(target_os = "android"))]
pub fn probe_list_audio_ports() { log::warn!("audio-caps ports: android-only build"); }

/// Task-76 #6 — enumerate device ports over binder (the routing core's source).
#[cfg(target_os = "android")]
pub fn enumerate_device_ports() -> Vec<crate::audio_routing::AudioDeviceCaps> {
    binder_path::enumerate_device_ports()
}
#[cfg(not(target_os = "android"))]
pub fn enumerate_device_ports() -> Vec<crate::audio_routing::AudioDeviceCaps> { Vec::new() }

/// Routing write probe (`--probe-audio-policy-route <speaker|earpiece>`):
/// drives the COMMUNICATION force-use then restores it.
#[cfg(target_os = "android")]
pub fn probe_route(speaker: bool) { binder_path::probe_route(speaker); }
#[cfg(not(target_os = "android"))]
pub fn probe_route(_speaker: bool) { log::warn!("audio-policy route: android-only build"); }

/// wart-arbiter-audio M3 — set the global audio mode (comms session start/end).
#[cfg(target_os = "android")]
pub fn set_mode(comm: bool) { binder_path::set_mode(comm); }
#[cfg(not(target_os = "android"))]
pub fn set_mode(_comm: bool) {}

/// wart-arbiter-audio M3 — set the communication routing (speaker/earpiece).
#[cfg(target_os = "android")]
pub fn set_route(speaker: bool) { binder_path::set_route(speaker); }
#[cfg(not(target_os = "android"))]
pub fn set_route(_speaker: bool) {}

/// Task-76 P8 — apply a media-volume step on the arbiter-chosen device
/// (`speaker` = loudspeaker, else earpiece). The host applier.
#[cfg(target_os = "android")]
pub fn adjust_volume_on(speaker: bool, up: bool) { binder_path::adjust_volume_on(speaker, up); }
#[cfg(not(target_os = "android"))]
pub fn adjust_volume_on(_speaker: bool, _up: bool) {}

/// Task-76 — apply output mute/unmute on the arbiter-chosen device.
#[cfg(target_os = "android")]
pub fn set_media_mute(speaker: bool, muted: bool) { binder_path::set_media_mute(speaker, muted); }
#[cfg(not(target_os = "android"))]
pub fn set_media_mute(_speaker: bool, _muted: bool) {}

/// Task-76 P8 — forward a hardware VOLUME_UP(true)/DOWN(false) press to the
/// arbiter, the single volume decider. The arbiter picks the target device +
/// owner host and pushes back `audio-policy volume <dir> <dev>`. Forwarding
/// (rather than acting locally) dedups the key — the framework delivers it to
/// several wart surfaces, but only the arbiter-chosen host applies.
#[cfg(target_os = "android")]
pub fn forward_volume_key(up: bool) {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    // Include our pid so the arbiter can target this (live) host when there is
    // no Foreground slot (e.g. keyguard locked). During a call the arbiter
    // overrides this to the comms owner on the call route.
    let dir = if up { "up" } else { "down" };
    let line = format!("volume {dir} {}\n", std::process::id());
    match UnixStream::connect(crate::arbiter_sock::arbiter_sock_path()) {
        Ok(mut s) => { let _ = s.write_all(line.as_bytes()); let _ = s.flush(); }
        Err(e)    => log::warn!("audio: volume-key forward failed: {e} (arbiter down?)"),
    }
}
#[cfg(not(target_os = "android"))]
pub fn forward_volume_key(_up: bool) {}

/// Task 81 — forward a KEYCODE_POWER press to the arbiter (the single display-power
/// authority). Every host's InputReader sees the key under the ART-less path; the
/// arbiter dedups the fan-in and toggles the panel via setPowerMode. (Lives here
/// alongside `forward_volume_key` — the established host→arbiter key-forward spot.)
#[cfg(target_os = "android")]
pub fn forward_power_key() {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let line = format!("power-key {}\n", std::process::id());
    match UnixStream::connect(crate::arbiter_sock::arbiter_sock_path()) {
        Ok(mut s) => { let _ = s.write_all(line.as_bytes()); let _ = s.flush(); }
        Err(e)    => log::warn!("power: power-key forward failed: {e} (arbiter down?)"),
    }
}
#[cfg(not(target_os = "android"))]
pub fn forward_power_key() {}
