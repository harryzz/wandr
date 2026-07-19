//
//  FlyingNumber.swift
//  Memorizwift
//
//  Created by Molly Beach on 10/22/24.
//

import SwiftUI

struct FlyingNumber: View {
    let number: Int
    
    var body: some View {
        if number != 0 {
            Text(number, format: .number)
        }
    }
}

// [wandr] #Preview (Xcode canvas tooling) removed: freestanding macro, no library shim possible off-Apple. No runtime effect.
