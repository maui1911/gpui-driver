//! The `DriverExt` annotation API and the `DriverNode` wrapper element.

use gpui::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, HitboxBehavior,
    InspectorElementId, IntoElement, Pixels, SharedString, Window,
};

use crate::registry::{self, NodeRecord};

/// Annotates elements so the gpui-driver CLI can address them.
///
/// ```ignore
/// use gpui_driver::DriverExt;
///
/// div().id("save").on_click(...).driver_id("save_button")
/// ```
pub trait DriverExt: IntoElement + Sized + 'static {
    /// Registers this element's bounds and a hitbox under the given stable id on
    /// every driver inspection, making it addressable by `tree`/`click`/etc.
    fn driver_id(self, id: impl Into<SharedString>) -> DriverNode<Self> {
        DriverNode {
            id: id.into(),
            text: None,
            kind: short_type_name::<Self>(),
            element: Some(self),
        }
    }
}

impl<E: IntoElement + 'static> DriverExt for E {}

/// Wrapper element produced by [`DriverExt::driver_id`]. Transparent for layout and
/// painting; records bounds + hitbox into the driver registry during collection draws.
pub struct DriverNode<E> {
    id: SharedString,
    text: Option<SharedString>,
    kind: String,
    element: Option<E>,
}

impl<E> DriverNode<E> {
    /// Attaches a human-readable text label reported in the `tree` output.
    /// Use when the element's purpose isn't obvious from its id.
    pub fn driver_text(mut self, text: impl Into<SharedString>) -> Self {
        self.text = Some(text.into());
        self
    }
}

impl<E: IntoElement + 'static> Element for DriverNode<E> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let mut element = self
            .element
            .take()
            .expect("DriverNode requested layout twice")
            .into_any_element();
        (element.request_layout(window, cx), element)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        element: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let window_id = window.window_handle().window_id().as_u64();
        let registry = registry::global();
        let entered = if registry.is_collecting(window_id) {
            let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
            registry.enter(
                window_id,
                NodeRecord {
                    id: self.id.to_string(),
                    kind: self.kind.clone(),
                    text: self.text.as_ref().map(|t| t.to_string()),
                    bounds: convert_bounds(bounds),
                    parent: None, // assigned by the registry from the nesting stack
                    interactive: true,
                    hitbox: Some(hitbox),
                },
            )
        } else {
            None
        };

        element.prepaint(window, cx);

        if entered.is_some() {
            registry.exit(window_id);
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        element: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        element.paint(window, cx);
    }
}

impl<E: IntoElement + 'static> IntoElement for DriverNode<E> {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

pub(crate) fn convert_bounds(bounds: Bounds<Pixels>) -> gpui_driver_protocol::Bounds {
    gpui_driver_protocol::Bounds {
        x: f32::from(bounds.origin.x),
        y: f32::from(bounds.origin.y),
        w: f32::from(bounds.size.width),
        h: f32::from(bounds.size.height),
    }
}

/// `gpui::elements::div::Stateful<gpui::elements::div::Div>` -> `Stateful<Div>`.
fn short_type_name<T>() -> String {
    simplify_type_name(std::any::type_name::<T>())
}

fn simplify_type_name(full: &str) -> String {
    let mut out = String::with_capacity(full.len());
    let mut segment = String::new();
    for ch in full.chars() {
        match ch {
            '<' | '>' | ',' | ' ' => {
                out.push_str(&segment);
                segment.clear();
                if ch != ' ' {
                    out.push(ch);
                }
                if ch == ',' {
                    out.push(' ');
                }
            }
            ':' => segment.clear(),
            _ => segment.push(ch),
        }
    }
    out.push_str(&segment);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplifies_type_names() {
        assert_eq!(simplify_type_name("gpui::elements::div::Div"), "Div");
        assert_eq!(
            simplify_type_name("gpui::div::Stateful<gpui::div::Div>"),
            "Stateful<Div>"
        );
        assert_eq!(simplify_type_name("alloc::string::String"), "String");
    }
}
