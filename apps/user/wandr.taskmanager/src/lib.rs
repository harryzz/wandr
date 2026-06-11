//! wandr.taskmanager — a dioxus-canvas guest that renders the runtime's running
//! apps (task 92). It imports `wandr:task-manager/task-manager` (host-implemented:
//! forwarded to the arbiter + enriched with a `/proc/<pid>` sample), polls
//! `list-apps` + `system-mem` on a timer from the `pre_frame` hook, and renders a
//! row per app (label, kind, state, CPU‰, mem) plus a kill button. Tapping kill
//! calls `task-manager/kill-app` (no-op for protected chrome/launcher).
//!
//! Polling model: `pre_frame` pumps the host snapshot into a thread-local `MODEL`
//! and `mark_dirty`s on change; `app()` calls `needs_update()` to stay armed and
//! reads `MODEL` each render (the Signal-guest bridge). `cpu-permille` is
//! delta-based so it reads 0 on the first poll and becomes meaningful from the
//! second.

use std::cell::RefCell;

use dioxus::prelude::*;

// One combined generate! for everything this guest talks to: the trimmed
// `my:skiko-gfx` world (canvas/paragraph/ime imports + renderer/frame-pacing
// exports) AND the `wandr:task-manager/task-manager` import (see wit/). A second
// generate! would conflict on `_rt`/`cabi_realloc`/the component-type section, so
// they share one. Same `export_macro_name` + `runtime_path` as dioxus-canvas's
// `skiko_world!`, so `wire!` finds the bindings and the export macro.
dioxus_canvas::__wit_bindgen::generate!({
    world: "taskmanager-ui-app",
    path: "wit",
    generate_all,
    pub_export_macro: true,
    export_macro_name: "__dioxus_canvas_export",
    runtime_path: "::dioxus_canvas::__wit_bindgen::rt",
});

dioxus_canvas::wasi_canvas_bindings!();

use wandr::task_manager::task_manager;
use wandr::task_manager::types;

// ── model (host snapshot, decoupled from the generated types) ────────────────

#[derive(Clone, Default, PartialEq)]
struct Row {
    app_id: String,
    label: String,
    pid: u32,
    kind: &'static str,    // "system" | "user"
    state: &'static str,   // "foreground" | "background" | "overlay" | "headless"
    cpu_permille: u32,
    rss_kb: u64,
    pss_kb: u64,
    threads: u32,
}

#[derive(Clone, Default)]
struct Mem {
    total_kb: u64,
    available_kb: u64,
    wandr_pss_kb: u64,
}

#[derive(Default)]
struct Model {
    rows: Vec<Row>,
    mem: Mem,
}

thread_local! {
    static MODEL: RefCell<Model> = RefCell::new(Model::default());
    /// Frames since the last host poll — gates the ~1.5 s refresh cadence so input
    /// bursts (taps) don't hammer `/proc` every frame.
    static SINCE_POLL: std::cell::Cell<u32> = const { std::cell::Cell::new(u32::MAX) };
}

fn kind_str(k: types::AppKind) -> &'static str {
    match k {
        types::AppKind::System => "system",
        types::AppKind::User => "user",
    }
}

fn state_str(s: types::AppState) -> &'static str {
    match s {
        types::AppState::Foreground => "fg",
        types::AppState::Background => "bg",
        types::AppState::Overlay => "overlay",
        types::AppState::Headless => "headless",
    }
}

/// Drop the `wandr.` namespace prefix for display (`wandr.statusbar` → `statusbar`).
fn strip_ns(s: &str) -> &str {
    s.strip_prefix("wandr.").unwrap_or(s)
}

/// Pull a fresh snapshot from the host into `MODEL`. Returns whether it changed
/// (so the caller only `mark_dirty`s on a real delta — idle is a cheap poll).
fn poll() -> bool {
    let apps = task_manager::list_apps();
    let sm = task_manager::system_mem();
    let rows: Vec<Row> = apps
        .into_iter()
        .map(|a| Row {
            app_id: a.app_id,
            label: a.label,
            pid: a.pid,
            kind: kind_str(a.kind),
            state: state_str(a.state),
            cpu_permille: a.usage.cpu_permille,
            rss_kb: a.usage.mem_rss_kb,
            pss_kb: a.usage.mem_pss_kb,
            threads: a.usage.threads,
        })
        .collect();
    let mem = Mem {
        total_kb: sm.total_kb,
        available_kb: sm.available_kb,
        wandr_pss_kb: sm.wandr_pss_kb,
    };
    MODEL.with(|m| {
        let mut m = m.borrow_mut();
        // Cheap change check: row count + the per-row pid/cpu/pss fingerprint.
        let changed = m.rows.len() != rows.len()
            || m.rows.iter().zip(&rows).any(|(a, b)| {
                a.pid != b.pid
                    || a.cpu_permille != b.cpu_permille
                    || a.pss_kb != b.pss_kb
                    || a.state != b.state
            })
            || m.mem.available_kb != mem.available_kb;
        m.rows = rows;
        m.mem = mem;
        changed
    })
}

// How many frames between host polls. The host clamps the idle frame delay to
// ~1 s, so at the idle `set_min_frame_delay` below this is ~1.5 s between polls;
// during input bursts it skips polling until the counter rolls over.
const POLL_EVERY_FRAMES: u32 = 1;

dioxus_canvas::wire_wasi_canvas!(app, pre_frame: |r| {
    r.set_scale(1.5); // hi-dpi panel — author px are small; 1.5× is the readable size
                      // that still leaves the name column room (2.0 crowds the name into
                      // the usage column). Usage + kill are flex-shrink:0, name is
                      // min-width:0/overflow:hidden so the kill button never clips.
    let n = SINCE_POLL.with(|c| c.get().wrapping_add(1));
    if n >= POLL_EVERY_FRAMES {
        SINCE_POLL.with(|c| c.set(0));
        if poll() {
            r.mark_dirty();
        }
    } else {
        SINCE_POLL.with(|c| c.set(n));
    }
    // Slow steady cadence — a task manager refreshes ~1×/s, not every frame.
    // Input (taps) drives its own immediate frames, so the list stays responsive.
    r.set_min_frame_delay(1500);
});

// ── UI ───────────────────────────────────────────────────────────────────────

const BG: &str = "#12121A";
const CARD: &str = "#1F1F33";
const SUBTLE: &str = "#2A2A44";
const TEXT: &str = "#FFFFFF";
const MUTED: &str = "#C7C7D9";
const SYS: &str = "#4285F4";   // system-app badge
const USR: &str = "#34A853";   // user-app badge
const KILL: &str = "#EA4335";

/// KiB → a compact "12.3 MB" / "512 KB" string.
fn fmt_kb(kb: u64) -> String {
    if kb >= 1024 * 1024 {
        format!("{:.1} GB", kb as f64 / (1024.0 * 1024.0))
    } else if kb >= 1024 {
        format!("{:.0} MB", kb as f64 / 1024.0)
    } else {
        format!("{kb} KB")
    }
}

fn app() -> Element {
    // Stay armed: re-schedule this scope every render so the host snapshot pumped
    // into MODEL by `poll` (+ flagged via `mark_dirty`) reaches the tree.
    dioxus::core::needs_update();

    let (rows, mem) = MODEL.with(|m| {
        let m = m.borrow();
        (m.rows.clone(), m.mem.clone())
    });

    let used_kb = mem.total_kb.saturating_sub(mem.available_kb);
    let used_pct = if mem.total_kb > 0 {
        (used_kb as f64 / mem.total_kb as f64 * 100.0).round() as i32
    } else {
        0
    };
    let mem_line = format!(
        "{} / {} used · {} free · wandr {}",
        fmt_kb(used_kb),
        fmt_kb(mem.total_kb),
        fmt_kb(mem.available_kb),
        fmt_kb(mem.wandr_pss_kb),
    );

    rsx! {
        div {
            style: "display:flex; flex-direction:column; padding:40px; gap:28px; background:{BG}; height:100%;",

            // Header + memory gauge.
            div {
                style: "display:flex; flex-direction:column; gap:16px;",
                div {
                    style: "display:flex; flex-direction:row; align-items:center; justify-content:space-between;",
                    div { style: "color:{TEXT}; font-size:56px; font-weight:700;", "Task Manager" }
                    div { style: "color:{MUTED}; font-size:34px;", "{rows.len()} apps" }
                }
                div { style: "color:{MUTED}; font-size:28px;", "{mem_line}" }
                div {
                    style: "display:flex; flex-direction:row; height:20px; border-radius:10px; background:{SUBTLE};",
                    div { style: format!("height:20px; border-radius:10px; background:{}; width:{}%;", SYS, used_pct) }
                }
            }

            // App rows.
            div {
                style: "display:flex; flex-direction:column; overflow:scroll; flex-grow:1; gap:14px;",
                div {
                    style: "display:flex; flex-direction:column; flex-shrink:0; gap:14px;",
                    for row in rows.iter().cloned() {
                        AppRow { row }
                    }
                }
            }
        }
    }
}

#[component]
fn AppRow(row: Row) -> Element {
    let badge = if row.kind == "system" { SYS } else { USR };
    let cpu = format!("{}.{}%", row.cpu_permille / 10, row.cpu_permille % 10);
    let kill_id = row.app_id.clone();
    let name = strip_ns(&row.label).to_string();
    let app_id = strip_ns(&row.app_id).to_string();
    rsx! {
        div {
            style: "display:flex; flex-direction:row; align-items:center; gap:20px; background:{CARD}; padding:24px; border-radius:20px; flex-shrink:0;",

            // Kind badge.
            div {
                style: format!(
                    "display:flex; justify-content:center; align-items:center; width:72px; height:72px; border-radius:18px; background:{};",
                    badge
                ),
                div { style: "color:{TEXT}; font-size:30px; font-weight:700;",
                    "{row.kind.chars().next().unwrap_or('?').to_uppercase()}" }
            }

            // Name + identity. `min-width:0` lets this flex item shrink (yield
            // space to the fixed usage + kill columns) instead of overflowing.
            div {
                style: "display:flex; flex-direction:column; gap:6px; flex-grow:1; min-width:0; overflow:hidden;",
                div { style: "color:{TEXT}; font-size:36px; font-weight:600;", "{name}" }
                div { style: "color:{MUTED}; font-size:24px;", "{app_id} · pid {row.pid} · {row.state}" }
            }

            // Resource sample — fixed column, never squeezed.
            div {
                style: "display:flex; flex-direction:column; gap:6px; align-items:flex-end; width:200px; flex-shrink:0;",
                div { style: "color:{TEXT}; font-size:30px;", "CPU {cpu}" }
                div { style: "color:{MUTED}; font-size:24px;", "PSS {fmt_kb(row.pss_kb)} · {row.threads} thr" }
            }

            // Kill button — fixed, never squeezed off-screen.
            button {
                style: "display:flex; justify-content:center; align-items:center; width:88px; height:72px; border-radius:18px; background:{KILL}; flex-shrink:0;",
                onclick: move |_| { let _ = task_manager::kill_app(&kill_id); },
                div { style: "color:{TEXT}; font-size:28px; font-weight:700;", "End" }
            }
        }
    }
}
