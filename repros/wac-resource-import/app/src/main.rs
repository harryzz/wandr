wit_bindgen::generate!({ world: "app", path: "wit", generate_all });

fn main() {
    println!("composite says: {}", crate::test::rescheck::probe::run());
}
