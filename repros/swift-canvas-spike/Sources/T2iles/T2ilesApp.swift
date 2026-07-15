//
//  T2ilesApp.swift
//  T2iles
//
//  Created by Astemir Eleev on 25.07.2020.
//  Copyright © 2020 Astemir Eleev. All rights reserved.
//

import SwiftUI

// [wandr] The real app entry. Under -DWANDR_HEADLESS the deterministic teardown-repro driver in
// WandrHeadless.swift owns @main instead (temporary; remove the flag once the UAF is fixed & verified).
#if !WANDR_HEADLESS
@main
struct T2ilesApp: App {
    
    private var mainView: some View {
        let rawValue = UserDefaults.standard.integer(forKey: Notification.Name.gameBoardSize.rawValue)
        let boardSize = BoardSize(rawValue: rawValue) ?? BoardSize.fourByFour
        let initialBoardSizeRawValue = boardSize.rawValue
        
        return CompositeView(board: GameLogic(size: initialBoardSizeRawValue))
    }
    
    var body: some Scene {
        WindowGroup {
            mainView
        }
    }
}
#endif
