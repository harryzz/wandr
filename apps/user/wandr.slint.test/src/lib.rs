//! wandr.slint.test — Slint UI over the wandr canvas WIT (task 100 M4).
//!
//! The `.slint` markup is compiled by the `slint!` macro; the wandr platform
//! and the renderer/frame-pacing exports come from `slint_wandr::launch!`.
//! Default font: Slint embeds Inter as the universal wasm fallback
//! (i-slint-common sharedfontique), so no font registration is needed.

slint::slint! {
    import { Button, LineEdit, ListView, Slider, CheckBox, VerticalBox, HorizontalBox } from "std-widgets.slint";

    export component MainWindow inherits Window {
        background: #101418;
        in-out property <int> counter: 0;

        VerticalBox {
            spacing: 8px;

            Text {
                text: "slint-wandr";
                font-size: 28px;
                font-weight: 700;
                color: white;
            }
            Text {
                text: "Skia over WIT — task 100 M4";
                font-size: 14px;
                color: #99aadd;
            }

            HorizontalBox {
                spacing: 8px;
                Button {
                    text: "+1";
                    clicked => { root.counter += 1; }
                }
                Text {
                    text: "counter: " + root.counter;
                    font-size: 18px;
                    color: white;
                    vertical-alignment: center;
                }
            }

            LineEdit {
                placeholder-text: "type here…";
            }

            HorizontalBox {
                spacing: 8px;
                Slider { minimum: 0; maximum: 100; value: 40; }
                CheckBox { text: "check me"; }
            }

            // Gradient fill + rounded corners + drop shadow + rotation-free
            // opacity animation — the M2 effects in one widget.
            Rectangle {
                height: 64px;
                border-radius: 14px;
                background: @linear-gradient(90deg, #ff8a00, #e52e71);
                drop-shadow-blur: 14px;
                drop-shadow-color: #000000aa;
                drop-shadow-offset-y: 5px;

                pulse := Rectangle {
                    width: 22px;
                    height: 22px;
                    x: parent.width - 34px;
                    y: 21px;
                    border-radius: 11px;
                    background: white;
                    opacity: 0.15;
                    states [
                        bright when mod(root.counter, 2) == 0: {
                            opacity: 1.0;
                            in { animate opacity { duration: 600ms; } }
                            out { animate opacity { duration: 600ms; } }
                        }
                    ]
                }
                Text {
                    text: "gradient + shadow";
                    color: white;
                    font-size: 16px;
                    font-weight: 600;
                    vertical-alignment: center;
                    horizontal-alignment: center;
                }
            }

            // Scroll: drag the list (Flickable under ListView).
            ListView {
                for i in 40: Text {
                    text: "row " + i + " — the quick brown fox";
                    font-size: 14px;
                    color: i == 0 ? #ffd166 : #cccccc;
                    height: 28px;
                }
            }
        }
    }
}

slint_wandr::launch!(|| {
    let ui = MainWindow::new().expect("slint-wandr: create MainWindow");
    ui.show().expect("slint-wandr: show");
    ui
});
