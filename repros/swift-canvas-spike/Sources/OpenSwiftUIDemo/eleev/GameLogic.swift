//
//  GameLogic.swift
//  T2iles
//
//  Created by Astemir Eleev on 03.05.2020.
//  Copyright © 2020 Astemir Eleev. All rights reserved.
//

import OpenSwiftUI
import Foundation
import OpenCombine

// [determinism] fixed-seed xorshift so tile spawns are reproducible run-to-run.
struct WandrDetRNG: RandomNumberGenerator {
    var state: UInt64 = 0x9E37_79B9_7F4A_7C15
    mutating func next() -> UInt64 {
        state ^= state << 13; state ^= state >> 7; state ^= state << 17
        return state
    }
}
nonisolated(unsafe) var wandrDetRNG = WandrDetRNG()

// wandr: no-op stand-in for OpenCombine's ObservableObjectPublisher. GameLogic is NO LONGER
// ObservableObject — OpenCombine's publisher/AnyCancellable allocate a C++ UnfairLock that
// corrupts the Swift runtime's exclusivity state in the wasi reactor (works only in a command/
// main). Reactivity is driven by the reactor via @State instead; objectWillChange.send() is kept
// as a no-op so eleev's call sites compile unchanged.
struct WandrNoOpWillChange { func send() {} }

final class GameLogic {

    let objectWillChange = WandrNoOpWillChange()
        
    // MARK: - Typealiases
    
    typealias TileMatrixType = TileMatrix<IdentifiedTile>
    
    // MARK: - Publishd Properties
    
    private(set) var noPossibleMove: Bool = false
    private(set) var score: Int = 0
    private(set) var mergeMultiplier: Int = 0
    private(set) var boardSize: Int
    private(set) var hasMoveMergedTiles: Bool = false
    
    private(set) var lastGestureDirection: Direction = .up

    private let mergeMultiplierStep: Int = 2
    private var instanceId = 0
    private var mutableInstanceId: Int {
        instanceId += 1
        return instanceId
    }
    private var tileMatrix: TileMatrixType!
    
    var tiles: TileMatrixType {
        return tileMatrix
    }

    // MARK: - Initializers
    
    init(size: Int) {
        boardSize = size
        reset(boardSize: size)
        // wandr: dropped the OpenCombine NotificationCenter board-resize subscription (inert here,
        // and its AnyCancellable/Set allocate the C++ lock that corrupts the reactor runtime).
    }
    
    func reset() {
        reset(boardSize: boardSize)
    }
    
    func resetLastGestureDirection() {
        lastGestureDirection = .up
        objectWillChange.send()
    }
    
    enum State {
        case moved
        case merged
        case none
    }
    
    @discardableResult
    func move(_ direction: Direction) -> State {
        defer { objectWillChange.send() }
        defer { OperationQueue.main.addOperation { self.resetLastGestureDirection() } }
        
        lastGestureDirection = direction

        var moved = false
        var hasMergedBlocks: Bool = false

        let axis = direction == .left || direction == .right
        let previousMatrixSnapshot = tileMatrix
        
        for row in 0..<boardSize {
            var rowSnapshot = [IdentifiedTile?]()
            var compactRow = [IdentifiedTile]()
           
            computeIntermediateSnapshot(
                &rowSnapshot,
                &compactRow,
                axis: axis,
                currentRow: row
            )
            
            if merge(blocks: &compactRow, reverse: direction == .down || direction == .right) {
                hasMergedBlocks = true
            }
            
            var newRow = [IdentifiedTile?]()
            compactRow.forEach { newRow.append($0) }

            if compactRow.count < boardSize {
                nilout(rowCount: newRow.count, direction: direction, row: &newRow)
            }

            newRow.enumerated().forEach {
                if rowSnapshot[$0]?.value != $1?.value {
                    moved = true
                }
                tileMatrix.add($1, to: axis ? ($0, row) : (row, $0))
            }
        }
        return finalizeMove(
            previousMatrixSnapshot,
            hasMoved: moved,
            hasMergedBlocks: hasMergedBlocks
        )
    }
    
    // MARK: - Private Methods
    
    private func computeIntermediateSnapshot(
        _ rowSnapshot: inout [IdentifiedTile?],
        _ compactRow: inout [IdentifiedTile],
        axis: Bool,
        currentRow row: Int
    ) {
        for col in 0..<boardSize {
            if let block = tileMatrix[axis ? (col, row) : (row, col)] {
                rowSnapshot.append(block)
                compactRow.append(block)
            }
            rowSnapshot.append(nil)
        }
    }
    
    private func nilout(rowCount: Int, direction: Direction, row: inout [IdentifiedTile?]) {
        for _ in 0..<(boardSize - rowCount) {
            if direction == .left || direction == .up {
                row.append(nil)
            } else {
                row.insert(nil, at: 0)
            }
        }
    }
    
    private func finalizeMove(_ previousMatrixSnapshot: TileMatrixType?, hasMoved moved: Bool, hasMergedBlocks: Bool) -> State {
        let areEqual = previousMatrixSnapshot?.equals(to: tileMatrix)
        
        if moved && !(areEqual!) {
            var result: State = .moved
            
            if hasMergedBlocks == false {
                self.mergeMultiplier = 0
                result = .merged
            }
            
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.025 * TimeInterval(self.boardSize)) {
                self.generateBlocks(generator: .single)
                self.hasMoveMergedTiles = hasMergedBlocks
            }
            return result
        } else {
            let isMovePossible = previousMatrixSnapshot?.isMovePossible()
            
            if let isMovePossible = isMovePossible, isMovePossible == false {
                self.noPossibleMove = true
            }
            return .none
        }
    }
    
    private func merge(blocks: inout [IdentifiedTile], reverse: Bool) -> Bool {
        var hasMerged: Bool = false
        if reverse {
            blocks = blocks.reversed()
        }
        
        blocks = blocks
            .map { (false, $0) }
            .reduce([(Bool, IdentifiedTile)]()) { acc, item in
                if acc.last?.0 == false && acc.last?.1.value == item.1.value {
                    var accPrefix = Array(acc.dropLast())
                    var mergedBlock = item.1
                    mergedBlock.value *= 2
                    accPrefix.append((true, mergedBlock))
                    
                    self.mergeMultiplier += self.mergeMultiplierStep
                    self.score += (self.mergeMultiplier * mergedBlock.value)
                    hasMerged = true
                    
                    return accPrefix
                } else {
                    var accTmp = acc
                    accTmp.append((false, item.1))
                    return accTmp
                }
            }
            .map { $0.1 }
        
        if reverse {
            blocks = blocks.reversed()
        }
        return hasMerged
    }
    
    private func reset(boardSize: Int) {
        self.boardSize = boardSize
        tileMatrix = TileMatrixType(size: boardSize)
        resetLastGestureDirection()
        generateBlocks(generator: .double)
        score = 0
        mergeMultiplier = 0
        objectWillChange.send()
    }
    
    private enum TileGenerator {
        case single
        case double
    }
    
    @discardableResult
    private func generateBlocks(generator: TileGenerator) -> Bool {
        var blankLocations = [IndexPair]()
        
        for rowIndex in 0..<boardSize {
            for colIndex in 0..<boardSize {
                let index = (colIndex, rowIndex)
                
                if tileMatrix[index] == nil {
                    blankLocations.append(index)
                }
            }
        }

        defer {
            objectWillChange.send()
        }
                
        switch generator {
        case .single:
            return generateBlock(in: blankLocations)
        case .double:
            return generateBlockPair(in: blankLocations)
        }
    }
    
    private func generateBlock(in blankLocations: [IndexPair]) -> Bool {
        // [determinism] seed all tile-spawn randomness so the crash is reproducible run-to-run
        // (otherwise the victim Subgraph address moves every run and can't be traced).
        guard blankLocations.count >= 1 else {
            return false
        }
        let placeLocIndex = Int.random(in: 0..<blankLocations.count, using: &wandrDetRNG)
        tileMatrix.add(IdentifiedTile(id: mutableInstanceId,
                                        value: (((0...4).randomElement(using: &wandrDetRNG) ?? 0) == 0) ? 4 : 2),
                         to: blankLocations[placeLocIndex])
        return true
    }
    
    private func generateBlockPair(in blankLocations: [IndexPair]) -> Bool {
        guard generateBlock(in: blankLocations) else {
            return false
        }
        guard let lastLoc = blankLocations.last else {
            return false
        }
        
        var placeLocIndex = Int.random(in: 0..<blankLocations.count, using: &wandrDetRNG)
        var blankLocations = blankLocations
        blankLocations[placeLocIndex] = lastLoc
        placeLocIndex = Int.random(in: 0..<(blankLocations.count - 1), using: &wandrDetRNG)
        tileMatrix.add(
            IdentifiedTile(
                id: mutableInstanceId,
                value: 2),
            to: blankLocations[placeLocIndex]
        )
        return true
    }
}
