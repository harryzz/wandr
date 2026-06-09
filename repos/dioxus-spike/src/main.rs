//! Feasibility probe: dioxus-core (reactive VirtualDom + rsx) + taffy
//! (flexbox layout) on wasm32-wasip2. Proves the two crates compile for
//! our target and sketches the integration shape a real canvas-WIT
//! painter would build on: VirtualDom -> mutations, taffy -> layout.

use dioxus::prelude::*;

/// A tiny reactive component — the kind of thing a wandr Rust guest would
/// author instead of hand-rolling draw calls.
fn app() -> Element {
    let mut count = use_signal(|| 0);
    rsx! {
        div {
            div { "wandr launcher (dioxus spike)" }
            div {
                div { "tile A" }
                div { "tile B" }
                div { "count: {count}" }
            }
            button { onclick: move |_| count += 1, "tap" }
        }
    }
}

fn main() {
    // 1. Drive the reactive VirtualDom and pull the initial mutation list
    //    (this is what a custom renderer consumes to build its tree).
    let mut vdom = VirtualDom::new(app);
    let mutations = vdom.rebuild_to_vec();
    println!("dioxus: initial rebuild produced {} edit(s)", mutations.edits.len());

    // 2. Build a taffy fl/ex layout tree (what the renderer would derive
    //    from the element tree) and compute geometry against the panel.
    let mut taffy: taffy::TaffyTree<()> = taffy::TaffyTree::new();
    let tile_a = taffy
        .new_leaf(taffy::Style {
            size: taffy::Size {
                width: taffy::Dimension::Length(132.0),
                height: taffy::Dimension::Length(132.0),
            },
            ..Default::default()
        })
        .unwrap();
    let tile_b = taffy
        .new_leaf(taffy::Style {
            size: taffy::Size {
                width: taffy::Dimension::Length(132.0),
                height: taffy::Dimension::Length(132.0),
            },
            ..Default::default()
        })
        .unwrap();
    let row = taffy
        .new_with_children(
            taffy::Style {
                display: taffy::Display::Flex,
                gap: taffy::Size {
                    width: taffy::LengthPercentage::Length(36.0),
                    height: taffy::LengthPercentage::Length(36.0),
                },
                ..Default::default()
            },
            &[tile_a, tile_b],
        )
        .unwrap();
    taffy
        .compute_layout(
            row,
            taffy::Size {
                width: taffy::AvailableSpace::Definite(1440.0),
                height: taffy::AvailableSpace::Definite(2880.0),
            },
        )
        .unwrap();

    let la = taffy.layout(tile_a).unwrap();
    let lb = taffy.layout(tile_b).unwrap();
    println!(
        "taffy: tile_a @ ({}, {}) {}x{}; tile_b @ ({}, {}) {}x{}",
        la.location.x, la.location.y, la.size.width, la.size.height,
        lb.location.x, lb.location.y, lb.size.width, lb.size.height,
    );
}
