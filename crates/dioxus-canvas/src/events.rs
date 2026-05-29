//! Event bridge. dioxus-html dispatches every typed event (`onclick`, …) through
//! a global [`HtmlEventConverter`] that a renderer MUST install via
//! [`set_event_converter`] — otherwise the first event panics (`unwrap` on a
//! `None` converter). The event payload handed to `VirtualDom::handle_event`
//! must be an `Rc<PlatformEventData>` whose inner `Box<dyn Any>` the converter
//! downcasts. This is the same contract dioxus-web / dioxus-tui satisfy.
//!
//! The flat demo only fires pointer `click`s, so we implement `convert_mouse_data`
//! fully and leave the exotic event types `unimplemented!` until a guest needs
//! them (keyboard arrives with the first text-input dioxus guest).

use std::any::Any;
use std::rc::Rc;

use dioxus_html::geometry::{ClientPoint, ElementPoint, PagePoint, ScreenPoint};
use dioxus_html::input_data::{MouseButton, MouseButtonSet};
use dioxus_html::point_interaction::{
    InteractionElementOffset, InteractionLocation, ModifiersInteraction, PointerInteraction,
};
use dioxus_html::{
    set_event_converter, AnimationData, CancelData, ClipboardData, CompositionData, DragData,
    FocusData, FormData, HasMouseData, HtmlEventConverter, ImageData, KeyboardData, MediaData,
    Modifiers, MountedData, MouseData, PlatformEventData, PointerData, ResizeData, ScrollData,
    SelectionData, ToggleData, TouchData, TransitionData, VisibleData, WheelData,
};

/// A pointer event. `x,y` are surface-absolute (client/screen/page); `ex,ey`
/// are relative to the target element's box (`element_coordinates`) — sliders
/// and the HSV picker read the element-relative offset to derive their value.
struct ClickData {
    x: f64,
    y: f64,
    ex: f64,
    ey: f64,
}

impl InteractionLocation for ClickData {
    fn client_coordinates(&self) -> ClientPoint {
        ClientPoint::new(self.x, self.y)
    }
    fn screen_coordinates(&self) -> ScreenPoint {
        ScreenPoint::new(self.x, self.y)
    }
    fn page_coordinates(&self) -> PagePoint {
        PagePoint::new(self.x, self.y)
    }
}
impl InteractionElementOffset for ClickData {
    fn element_coordinates(&self) -> ElementPoint {
        ElementPoint::new(self.ex, self.ey)
    }
}
impl ModifiersInteraction for ClickData {
    fn modifiers(&self) -> Modifiers {
        Modifiers::empty()
    }
}
impl PointerInteraction for ClickData {
    fn trigger_button(&self) -> Option<MouseButton> {
        Some(MouseButton::Primary)
    }
    fn held_buttons(&self) -> MouseButtonSet {
        MouseButtonSet::empty()
    }
}
impl HasMouseData for ClickData {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Build the `Rc<dyn Any>` payload for a pointer event: `(x,y)` surface-absolute,
/// `(ex,ey)` element-relative.
pub fn mouse_event(x: f32, y: f32, ex: f32, ey: f32) -> Rc<dyn Any> {
    Rc::new(PlatformEventData::new(Box::new(ClickData {
        x: x as f64,
        y: y as f64,
        ex: ex as f64,
        ey: ey as f64,
    })))
}

struct WartEventConverter;

impl HtmlEventConverter for WartEventConverter {
    fn convert_mouse_data(&self, event: &PlatformEventData) -> MouseData {
        match event.downcast::<ClickData>() {
            Some(c) => MouseData::new(ClickData { x: c.x, y: c.y, ex: c.ex, ey: c.ey }),
            None => MouseData::new(ClickData { x: 0.0, y: 0.0, ex: 0.0, ey: 0.0 }),
        }
    }

    fn convert_animation_data(&self, _: &PlatformEventData) -> AnimationData {
        unimplemented!("dioxus-canvas: animation events unsupported")
    }
    fn convert_cancel_data(&self, _: &PlatformEventData) -> CancelData {
        unimplemented!("dioxus-canvas: cancel events unsupported")
    }
    fn convert_clipboard_data(&self, _: &PlatformEventData) -> ClipboardData {
        unimplemented!("dioxus-canvas: clipboard events unsupported")
    }
    fn convert_composition_data(&self, _: &PlatformEventData) -> CompositionData {
        unimplemented!("dioxus-canvas: composition events unsupported")
    }
    fn convert_drag_data(&self, _: &PlatformEventData) -> DragData {
        unimplemented!("dioxus-canvas: drag events unsupported")
    }
    fn convert_focus_data(&self, _: &PlatformEventData) -> FocusData {
        unimplemented!("dioxus-canvas: focus events unsupported")
    }
    fn convert_form_data(&self, _: &PlatformEventData) -> FormData {
        unimplemented!("dioxus-canvas: form events unsupported")
    }
    fn convert_image_data(&self, _: &PlatformEventData) -> ImageData {
        unimplemented!("dioxus-canvas: image events unsupported")
    }
    fn convert_keyboard_data(&self, _: &PlatformEventData) -> KeyboardData {
        unimplemented!("dioxus-canvas: keyboard events unsupported (no text-input guest yet)")
    }
    fn convert_media_data(&self, _: &PlatformEventData) -> MediaData {
        unimplemented!("dioxus-canvas: media events unsupported")
    }
    fn convert_mounted_data(&self, _: &PlatformEventData) -> MountedData {
        unimplemented!("dioxus-canvas: mounted events unsupported")
    }
    fn convert_pointer_data(&self, _: &PlatformEventData) -> PointerData {
        unimplemented!("dioxus-canvas: pointer events unsupported")
    }
    fn convert_resize_data(&self, _: &PlatformEventData) -> ResizeData {
        unimplemented!("dioxus-canvas: resize events unsupported")
    }
    fn convert_scroll_data(&self, _: &PlatformEventData) -> ScrollData {
        unimplemented!("dioxus-canvas: scroll events unsupported")
    }
    fn convert_selection_data(&self, _: &PlatformEventData) -> SelectionData {
        unimplemented!("dioxus-canvas: selection events unsupported")
    }
    fn convert_toggle_data(&self, _: &PlatformEventData) -> ToggleData {
        unimplemented!("dioxus-canvas: toggle events unsupported")
    }
    fn convert_touch_data(&self, _: &PlatformEventData) -> TouchData {
        unimplemented!("dioxus-canvas: touch events unsupported")
    }
    fn convert_transition_data(&self, _: &PlatformEventData) -> TransitionData {
        unimplemented!("dioxus-canvas: transition events unsupported")
    }
    fn convert_visible_data(&self, _: &PlatformEventData) -> VisibleData {
        unimplemented!("dioxus-canvas: visible events unsupported")
    }
    fn convert_wheel_data(&self, _: &PlatformEventData) -> WheelData {
        unimplemented!("dioxus-canvas: wheel events unsupported")
    }
}

/// Install the global converter. Idempotent-safe to call once at renderer init.
pub fn install() {
    set_event_converter(Box::new(WartEventConverter));
}
