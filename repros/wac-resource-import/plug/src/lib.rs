// Minimal repro: a reactor that imports the wandr:crypto/aead RESOURCE and is
// wac-plugged into a command app. Tests whether an imported WIT resource links
// through a wac composition (the wandr.signal failure suggested it doesn't).
wit_bindgen::generate!({ world: "plug", path: "wit", generate_all });

use crate::exports::test::rescheck::probe::Guest;
use crate::wandr::crypto::aead::AeadKey;
use crate::wandr::crypto::types::AeadAlgo;

struct C;
impl Guest for C {
    fn run() -> String {
        let k = AeadKey::create(AeadAlgo::Aes256Gcm, &[7u8; 32]).expect("create");
        let sealed = k.seal(&[0u8; 12], b"aad", b"resource-through-wac works").expect("seal");
        let pt = k.open(&[0u8; 12], b"aad", &sealed).expect("open");
        String::from_utf8(pt).unwrap()
    }
}
export!(C);
