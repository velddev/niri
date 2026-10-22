use niri_config::Config;
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    Anchor, KeyboardInteractivity,
};
use smithay::utils::Point;
use wayland_client::protocol::wl_surface::WlSurface;

use super::*;
use crate::layer::dismiss::DismissTarget;
use crate::tests::client::{ClientId, LayerConfigureProps};

const CONFIG: &str = r##"
layer-rule {
    match namespace="^card$"
    dismissable true
}
"##;

// One 1920×1080 output with a focused 100×100 window.
fn set_up() -> (Fixture, ClientId) {
    let config = Config::parse_mem(CONFIG).unwrap();
    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));

    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.set_size(100, 100);
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    (f, id)
}

// A 400×200 surface centred at the top of the output: x 760..1160, y 0..200.
fn map_layer(
    f: &mut Fixture,
    id: ClientId,
    layer: Layer,
    namespace: &str,
    kb: KeyboardInteractivity,
) -> WlSurface {
    let layer = f.client(id).create_layer(None, layer, namespace);
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Top),
        size: Some((400, 200)),
        kb_interactivity: Some(kb),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);

    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.set_size(400, 200);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    surface
}

fn card(f: &mut Fixture, id: ClientId) -> WlSurface {
    map_layer(f, id, Layer::Overlay, "card", KeyboardInteractivity::None)
}

fn format(targets: Vec<DismissTarget>) -> String {
    let names: Vec<_> = targets
        .iter()
        .map(|(surface, output)| format!("{} on {output}", surface.namespace()))
        .collect();
    names.join(", ")
}

fn on_press(f: &mut Fixture, x: f64, y: f64) -> String {
    format(f.niri().layers_to_dismiss_on_press(Point::from((x, y))))
}

fn on_escape(f: &mut Fixture) -> String {
    format(f.niri().layers_to_dismiss_on_escape())
}

#[test]
fn nothing_without_dismissable_layers() {
    let (mut f, id) = set_up();
    map_layer(
        &mut f,
        id,
        Layer::Overlay,
        "other",
        KeyboardInteractivity::None,
    );

    assert_eq!(on_press(&mut f, 960., 600.), "");
    assert_eq!(on_escape(&mut f), "");
}

#[test]
fn press_outside_dismisses() {
    let (mut f, id) = set_up();
    card(&mut f, id);
    let output = f.niri_output(1).name();

    // On the window, on empty space next to it, and just outside the card's corner.
    let want = format!("card on {output}");
    assert_eq!(on_press(&mut f, 960., 540.), want);
    assert_eq!(on_press(&mut f, 100., 900.), want);
    assert_eq!(on_press(&mut f, 1161., 201.), want);
}

#[test]
fn press_inside_does_nothing() {
    let (mut f, id) = set_up();
    card(&mut f, id);

    assert_eq!(on_press(&mut f, 960., 100.), "");
    assert_eq!(on_press(&mut f, 761., 1.), "");
}

#[test]
fn press_on_another_layer_dismisses() {
    let (mut f, id) = set_up();
    card(&mut f, id);
    // A bar on the top layer, full width at the bottom.
    let layer = f.client(id).create_layer(None, Layer::Top, "bar");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Left | Anchor::Right | Anchor::Bottom),
        size: Some((0, 40)),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);
    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.set_size(1920, 40);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    let output = f.niri_output(1).name();
    assert_eq!(on_press(&mut f, 960., 1060.), format!("card on {output}"));
}

#[test]
fn input_region_counts() {
    let (mut f, id) = set_up();
    let surface = card(&mut f, id);
    let output = f.niri_output(1).name();

    // Only the top half takes input: the bottom half is outside.
    f.client(id).set_input_region(&surface, &[(0, 0, 400, 100)]);
    f.client(id).layer(&surface).commit();
    f.double_roundtrip(id);
    assert_eq!(on_press(&mut f, 960., 50.), "");
    assert_eq!(on_press(&mut f, 960., 150.), format!("card on {output}"));

    // Fully click-through: not dismissable at the moment.
    f.client(id).set_input_region(&surface, &[]);
    f.client(id).layer(&surface).commit();
    f.double_roundtrip(id);
    assert_eq!(on_press(&mut f, 960., 600.), "");
    assert_eq!(on_escape(&mut f), "");
}

#[test]
fn escape_with_focus_elsewhere_dismisses() {
    let (mut f, id) = set_up();
    card(&mut f, id);
    let output = f.niri_output(1).name();

    assert!(f.niri().keyboard_focus.surface().is_some());
    assert_eq!(on_escape(&mut f), format!("card on {output}"));
}

#[test]
fn escape_with_focus_on_the_layer_goes_to_it() {
    let (mut f, id) = set_up();
    map_layer(
        &mut f,
        id,
        Layer::Overlay,
        "card",
        KeyboardInteractivity::Exclusive,
    );

    let focus = f.niri().keyboard_focus.surface().cloned();
    let mapped: Vec<_> = f
        .niri()
        .mapped_layer_surfaces
        .keys()
        .map(|l| l.wl_surface().clone())
        .collect();
    assert_eq!(focus.as_ref(), mapped.first());
    assert_eq!(mapped.len(), 1);

    assert_eq!(on_escape(&mut f), "");
    // A press outside still dismisses it.
    assert!(!on_press(&mut f, 960., 600.).is_empty());
}

#[test]
fn on_demand_focus_on_the_layer() {
    let (mut f, id) = set_up();
    map_layer(
        &mut f,
        id,
        Layer::Overlay,
        "card",
        KeyboardInteractivity::OnDemand,
    );
    let output = f.niri_output(1).name();

    // niri gives a newly mapped on-demand surface the keyboard: Escape is its own.
    assert_eq!(on_escape(&mut f), "");

    // A click elsewhere takes it away (what on_pointer_button does): now Escape dismisses.
    f.niri().focus_layer_surface_if_on_demand(None);
    f.niri_state().update_keyboard_focus();
    assert_eq!(on_escape(&mut f), format!("card on {output}"));

    // A click on it gives it back.
    let layer = f.niri().mapped_layer_surfaces.keys().next().cloned();
    f.niri().focus_layer_surface_if_on_demand(layer);
    f.niri_state().update_keyboard_focus();
    assert_eq!(on_escape(&mut f), "");
}

#[test]
fn unmapped_layer_is_forgotten() {
    let (mut f, id) = set_up();
    let surface = card(&mut f, id);

    let layer = f.client(id).layer(&surface);
    layer.attach_null();
    layer.commit();
    f.double_roundtrip(id);

    assert_eq!(on_press(&mut f, 960., 600.), "");
    assert_eq!(on_escape(&mut f), "");
}

#[test]
fn nothing_in_the_overview() {
    let (mut f, id) = set_up();
    card(&mut f, id);

    f.niri().layout.open_overview();
    assert_eq!(on_press(&mut f, 960., 600.), "");
    assert_eq!(on_escape(&mut f), "");

    f.niri().layout.close_overview();
    assert!(!on_escape(&mut f).is_empty());
}

#[test]
fn one_event_per_dismissable_layer() {
    let (mut f, id) = set_up();
    card(&mut f, id);
    // A second dismissable surface, bottom-left.
    let layer = f.client(id).create_layer(None, Layer::Top, "card");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Left | Anchor::Bottom),
        size: Some((300, 300)),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);
    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.set_size(300, 300);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    let output = f.niri_output(1).name();
    let both = format!("card on {output}, card on {output}");
    assert_eq!(on_press(&mut f, 1500., 600.), both);
    assert_eq!(on_escape(&mut f), both);
    // A press on one of them dismisses only the other.
    assert_eq!(on_press(&mut f, 100., 1000.), format!("card on {output}"));
    assert_eq!(on_press(&mut f, 960., 100.), format!("card on {output}"));
}
