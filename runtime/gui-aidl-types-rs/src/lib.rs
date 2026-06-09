//! gui_aidl_types_rs — rsbinder shim for the native-backed `android.gui`
//! parcelables (task 78). See Cargo.toml for the why. These are zero-sized stubs
//! whose marshalling is `unimplemented!()` — they exist only so the full
//! `ISurfaceComposer` binding compiles; the methods that use them are never
//! called (we only call `setPowerMode` / `getPhysicalDisplayToken` /
//! `setDisplayBrightness`, none of which reference these types). Mirrors AOSP's
//! own `gui_aidl_types_rs` (which is likewise `todo!()` stubs).

#[cfg(target_os = "android")]
mod stubs {
    use rsbinder::{Parcel, Parcelable, Result};

    /// Define a zero-sized stub parcelable wired into rsbinder's
    /// Serialize/Deserialize machinery, with `unimplemented!()` marshalling.
    macro_rules! stub_parcelable {
        ($name:ident) => {
            #[derive(Debug, Default, Clone, PartialEq)]
            pub struct $name(());

            impl Parcelable for $name {
                fn write_to_parcel(&self, _parcel: &mut Parcel) -> Result<()> {
                    unimplemented!(concat!(
                        stringify!($name),
                        ": gui_aidl_types_rs shim parcelable is not marshallable (wandr task 78)"
                    ))
                }
                fn read_from_parcel(&mut self, _parcel: &mut Parcel) -> Result<()> {
                    unimplemented!(concat!(
                        stringify!($name),
                        ": gui_aidl_types_rs shim parcelable is not marshallable (wandr task 78)"
                    ))
                }
            }

            rsbinder::impl_serialize_for_parcelable!($name);
            rsbinder::impl_deserialize_for_parcelable!($name);
        };
    }

    // The native-backed types referenced by android.gui.ISurfaceComposer (and
    // friends), matching AOSP's gui_aidl_types_rs set.
    stub_parcelable!(BitTube);
    stub_parcelable!(DisplayInfo);
    stub_parcelable!(LayerDebugInfo);
    stub_parcelable!(LayerMetadata);
    stub_parcelable!(ParcelableVsyncEventData);
    stub_parcelable!(ScreenCaptureResults);
    stub_parcelable!(VsyncEventData);
    stub_parcelable!(WindowInfo);
    stub_parcelable!(WindowInfosUpdate);
}

#[cfg(target_os = "android")]
pub use stubs::*;
