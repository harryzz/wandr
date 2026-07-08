//! Pure-p2 sync control component (task 115 M2a spike, kill-gate 4): proves a
//! normal wandr app still instantiates and calls SYNC on the same engine even
//! with `async_support` + `component-model-async` enabled in the config.
wit_bindgen::generate!({ world: "p2sync", path: "wit" });

struct P2;

impl Guest for P2 {
    fn run() -> u32 {
        // touch p2 wasi (stderr) so the component carries real p2 imports,
        // like every existing wandr guest.
        eprintln!("p2sync alive");
        7
    }
}

export!(P2);
