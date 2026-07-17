// [wandr Phase 2 · Audio shim] AudioToolbox stand-in for the AudioServices API eleev's Audio.swift
// uses (import AudioToolbox → AudioServicesCreateSystemSoundID / AudioServicesPlaySystemSound).
// Routes to WandrAudioPlayer (wasi:audio/pcm) — the same generic engine WandrRuntime itself uses.
import Foundation
import WandrRuntime

public typealias SystemSoundID = UInt32
public typealias OSStatus = Int32
// eleev calls `url as CFURL`; off-Apple Foundation has no URL↔CFURL bridge, so provide CFURL here.
public typealias CFURL = URL

nonisolated(unsafe) private var _nextSoundID: SystemSoundID = 1
nonisolated(unsafe) private var _soundNames: [SystemSoundID: String] = [:]

@discardableResult
public func AudioServicesCreateSystemSoundID(_ inFileURL: CFURL,
                                             _ outSystemSoundID: UnsafeMutablePointer<SystemSoundID>) -> OSStatus {
    let id = _nextSoundID; _nextSoundID &+= 1
    // WandrAudioPlayer.play(fileNamed:) takes the bare asset name (no extension, no path) — it
    // owns the `/assets/<name>.wav` convention itself.
    _soundNames[id] = inFileURL.deletingPathExtension().lastPathComponent
    outSystemSoundID.pointee = id
    return 0
}

public func AudioServicesPlaySystemSound(_ inSystemSoundID: SystemSoundID) {
    guard let name = _soundNames[inSystemSoundID] else { return }
    WandrAudioPlayer.shared.play(fileNamed: name)
}
