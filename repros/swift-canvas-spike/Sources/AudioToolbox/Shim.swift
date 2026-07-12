// [wandr Phase 2 · Audio shim] AudioToolbox stand-in for the AudioServices API eleev's Audio.swift
// uses (import AudioToolbox → AudioServicesCreateSystemSoundID / AudioServicesPlaySystemSound).
// v1: registers a sound id and plays SILENTLY so the app compiles + runs unmodified.
// TODO: route AudioServicesPlaySystemSound → wasi:audio (short PCM blip; gate on @AppStorage audio).
import Foundation

public typealias SystemSoundID = UInt32
public typealias OSStatus = Int32
// eleev calls `url as CFURL`; off-Apple Foundation has no URL↔CFURL bridge, so provide CFURL here.
public typealias CFURL = URL

nonisolated(unsafe) private var _nextSoundID: SystemSoundID = 1
nonisolated(unsafe) private var _soundURLs: [SystemSoundID: URL] = [:]

@discardableResult
public func AudioServicesCreateSystemSoundID(_ inFileURL: CFURL,
                                             _ outSystemSoundID: UnsafeMutablePointer<SystemSoundID>) -> OSStatus {
    let id = _nextSoundID; _nextSoundID &+= 1
    _soundURLs[id] = inFileURL
    outSystemSoundID.pointee = id
    return 0
}

public func AudioServicesPlaySystemSound(_ inSystemSoundID: SystemSoundID) {
    // v1: silent. TODO: decode _soundURLs[id] and play via wasi:audio.
}
