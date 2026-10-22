//! Dismissable layer surfaces (the `dismissable` layer rule).
//!
//! While one is mapped, a pointer press outside of it and an unbound Escape ask its client to
//! close it, with a `LayerDismissRequested` event on the IPC event stream. niri never closes the
//! surface itself and never moves keyboard focus for this: the press still goes where it went,
//! and Escape is only consumed when keyboard focus is somewhere else.

use niri_ipc::LayerDismissReason;
use smithay::desktop::{layer_map_for_output, LayerSurface};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point};
use smithay::wayland::compositor::{with_states, RectangleKind, SurfaceAttributes};

use crate::niri::{Niri, State};

/// A dismissable layer surface and the name of its output.
pub type DismissTarget = (LayerSurface, String);

impl Niri {
    /// Mapped layer surfaces with the `dismissable` rule that take pointer input somewhere.
    ///
    /// A surface with an empty input region (fully click-through, e.g. while it animates out)
    /// doesn't count.
    pub fn dismissable_layers(&self) -> Vec<DismissTarget> {
        let mut rv = Vec::new();
        for output in self.global_space.outputs() {
            let map = layer_map_for_output(output);
            for surface in map.layers() {
                let Some(mapped) = self.mapped_layer_surfaces.get(surface) else {
                    continue;
                };
                if !mapped.rules().dismissable {
                    continue;
                }
                if !accepts_pointer_input(surface.wl_surface()) {
                    continue;
                }
                rv.push((surface.clone(), output.name()));
            }
        }
        rv
    }

    /// Whether something else owns input right now, so dismissing must stay out of the way.
    fn layer_dismiss_suppressed(&self) -> bool {
        self.is_locked()
            || self.exit_confirm_dialog.is_open()
            || self.screenshot_ui.is_open()
            || self.window_mru_ui.is_open()
            || self.layout.is_overview_open()
            || self.popup_grab.is_some()
            // Interactive move/resize, a held button, a pop-up's pointer grab and so on.
            || self.seat.get_pointer().is_some_and(|pointer| pointer.is_grabbed())
    }

    /// Dismissable layers that a pointer press at `pos` (global coordinates) is outside of.
    pub fn layers_to_dismiss_on_press(&self, pos: Point<f64, Logical>) -> Vec<DismissTarget> {
        if self.layer_dismiss_suppressed() {
            return Vec::new();
        }

        let mut layers = self.dismissable_layers();
        if layers.is_empty() {
            return layers;
        }

        // The press lands on the surface itself or one of its pop-ups: not outside.
        let under = self.contents_under(pos).layer;
        layers.retain(|(surface, _)| under.as_ref() != Some(surface));
        layers
    }

    /// Dismissable layers that an unbound Escape press should dismiss.
    ///
    /// Empty when the keyboard focus is on one of them (it gets the Escape itself).
    pub fn layers_to_dismiss_on_escape(&self) -> Vec<DismissTarget> {
        if self.layer_dismiss_suppressed() {
            return Vec::new();
        }

        let layers = self.dismissable_layers();
        if layers.is_empty() {
            return layers;
        }

        if let Some(focus) = self.keyboard_focus.surface() {
            let root = self.find_root_shell_surface(focus);
            if layers
                .iter()
                .any(|(surface, _)| *surface.wl_surface() == root)
            {
                return Vec::new();
            }
        }

        layers
    }
}

impl State {
    /// Sends a `LayerDismissRequested` event for every target.
    pub fn request_layer_dismiss(
        &mut self,
        targets: Vec<DismissTarget>,
        reason: LayerDismissReason,
    ) {
        for (surface, output) in targets {
            let namespace = surface.namespace().to_owned();
            debug!("asking layer surface {namespace:?} on {output} to close ({reason:?})");
            self.ipc_layer_dismiss_requested(namespace, output, reason);
        }
    }
}

/// Whether the surface's input region is non-empty.
fn accepts_pointer_input(surface: &WlSurface) -> bool {
    with_states(surface, |states| {
        let mut cached = states.cached_state.get::<SurfaceAttributes>();
        match &cached.current().input_region {
            // No input region: the whole surface takes input.
            None => true,
            Some(region) => region
                .rects
                .iter()
                .any(|(kind, rect)| matches!(kind, RectangleKind::Add) && !rect.is_empty()),
        }
    })
}
