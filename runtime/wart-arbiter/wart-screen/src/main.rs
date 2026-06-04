// wart-screen — `wart-screen on|off` (task 81, ART-less display power).
//
// Toggles the primary panel via the shared wart-hal-display setPowerMode path. Run
// by the arbiter's power module through `wart-launch` (uid system) so the
// SurfaceFlinger permission check short-circuits with the Java framework stopped —
// the arbiter itself is plain root, which HANGS on that check under ART-off.
//
// Exit 0 = panel set, 1 = setPowerMode failed, 2 = bad usage.

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_default();
    let on = match arg.as_str() {
        "on" | "1" | "wake" => true,
        "off" | "0" | "sleep" => false,
        _ => {
            eprintln!("usage: wart-screen on|off");
            std::process::exit(2);
        }
    };
    let ok = wart_hal_display::set_display_power(on);
    println!("wart-screen: set_display_power({on}) = {ok}");
    std::process::exit(if ok { 0 } else { 1 });
}
