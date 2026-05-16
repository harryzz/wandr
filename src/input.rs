use wasmtime::Store;
use crate::HostState;
use crate::bindings::SkikoUi;
use crate::bindings::exports::my::skiko_gfx::renderer::{KeyKind, PointerKind};

pub fn dispatch_pointer(
    bindings: &SkikoUi,
    store: &mut Store<HostState>,
    kind: u8,
    x: f32, y: f32,
) -> anyhow::Result<()> {
    let kind = match kind {
        0 => PointerKind::Down,
        1 => PointerKind::Up,
        2 => PointerKind::Move,
        _ => PointerKind::Scroll,
    };
    bindings.my_skiko_gfx_renderer()
        .call_on_pointer_event(store, kind, x, y)?;
    Ok(())
}

/// Enriched dispatch: also delivers pointer-id (multi-touch) and pressure.
/// Calls both v1 (for backward compat) and v2 (for callers that want the
/// extras). Single-touch / mouse callers should pass pointer_id=0.
pub fn dispatch_pointer_v2(
    bindings: &SkikoUi,
    store: &mut Store<HostState>,
    kind: u8,
    pointer_id: u32,
    x: f32, y: f32,
    pressure: f32,
) -> anyhow::Result<()> {
    let pk = match kind {
        0 => PointerKind::Down,
        1 => PointerKind::Up,
        2 => PointerKind::Move,
        _ => PointerKind::Scroll,
    };
    let r = bindings.my_skiko_gfx_renderer();
    r.call_on_pointer_event(&mut *store, pk, x, y)?;
    r.call_on_pointer_event_v2(store, pointer_id, pk, x, y, pressure)?;
    Ok(())
}

pub fn dispatch_key(
    bindings: &SkikoUi,
    store: &mut Store<HostState>,
    kind: u8, key_code: u32,
) -> anyhow::Result<()> {
    let kind = if kind == 0 { KeyKind::Down } else { KeyKind::Up };
    bindings.my_skiko_gfx_renderer()
        .call_on_key_event(store, kind, key_code)?;
    Ok(())
}

/// Enriched key dispatch: carries the resolved UTF-32 codepoint AND a
/// Compose-compatible key-id. Hosts emit both v1 (`on-key-event`) and v2
/// (`on-key-event-v2`) for every keystroke. Guests can ignore whichever
/// they don't need.
pub fn dispatch_key_v2(
    bindings: &SkikoUi,
    store: &mut Store<HostState>,
    kind: u8, code_point: u32, key_id: u32,
) -> anyhow::Result<()> {
    let kk = if kind == 0 { KeyKind::Down } else { KeyKind::Up };
    bindings.my_skiko_gfx_renderer()
        .call_on_key_event_v2(store, kk, code_point, key_id)?;
    Ok(())
}

pub fn dispatch_resize(
    bindings: &SkikoUi,
    store: &mut Store<HostState>,
    w: u32, h: u32,
) -> anyhow::Result<()> {
    bindings.my_skiko_gfx_renderer()
        .call_on_resize(store, w, h)?;
    Ok(())
}
