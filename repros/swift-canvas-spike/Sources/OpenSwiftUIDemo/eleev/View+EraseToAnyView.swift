//
//  View+EraseToAnyView.swift
//  T2iles
//
//  Created by Astemir Eleev on 03.05.2020.
//  Copyright © 2020 Astemir Eleev. All rights reserved.
//

import OpenSwiftUI
import Foundation

extension View {
    
    var eraseToAnyView: AnyView {
        return AnyView(self)
    }
    
}
