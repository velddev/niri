use niri_config::Config;
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::{
    Anchor, KeyboardInteractivity,
};
use smithay::utils::Point;
use wayland_client::protocol::wl_surface::WlSurface;

use super::*;
use crate::niri::KeyboardFocus;
use crate::render_helpers::RenderTarget;
use crate::tests::client::{ClientId, LayerConfigureProps};

// The card is dismissable and blocked out from screencasts, like pond-shell's Ask Miki card.
const CONFIG: &str = r##"
layer-rule {
    match namespace="^card$"
    dismissable true
    block-out-from "screencast"
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

// A layer surface anchored to the top of the output, mapped at the given size.
fn map_layer(
    f: &mut Fixture,
    id: ClientId,
    namespace: &str,
    size: (u16, u16),
    kb: KeyboardInteractivity,
) -> WlSurface {
    let layer = f.client(id).create_layer(None, Layer::Overlay, namespace);
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Top),
        size: Some((u32::from(size.0), u32::from(size.1))),
        kb_interactivity: Some(kb),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);

    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.set_size(size.0, size.1);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    surface
}

// A kept card while idle: 1×1, no input region, no keyboard.
fn kept_card(f: &mut Fixture, id: ClientId) -> WlSurface {
    let surface = map_layer(f, id, "card", (1, 1), KeyboardInteractivity::None);
    f.client(id).set_input_region(&surface, &[]);
    f.client(id).layer(&surface).commit();
    f.double_roundtrip(id);
    surface
}

// Switches the keyboard interactivity of a mapped surface (one commit, nothing else changes).
fn set_kb(f: &mut Fixture, id: ClientId, surface: &WlSurface, kb: KeyboardInteractivity) {
    let layer = f.client(id).layer(surface);
    layer.set_configure_props(LayerConfigureProps {
        kb_interactivity: Some(kb),
        ..Default::default()
    });
    layer.commit();
    f.double_roundtrip(id);
}

// Resizes a mapped surface, in one commit with the given keyboard interactivity (if any), then
// acks the configure with a buffer of the new size.
fn resize(
    f: &mut Fixture,
    id: ClientId,
    surface: &WlSurface,
    size: (u16, u16),
    kb: Option<KeyboardInteractivity>,
) {
    let layer = f.client(id).layer(surface);
    layer.set_configure_props(LayerConfigureProps {
        size: Some((u32::from(size.0), u32::from(size.1))),
        kb_interactivity: kb,
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);

    let layer = f.client(id).layer(surface);
    layer.attach_new_buffer();
    layer.set_size(size.0, size.1);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);
}

// Commits a mapped surface again with nothing changed (a new frame).
fn redraw(f: &mut Fixture, id: ClientId, surface: &WlSurface) {
    let layer = f.client(id).layer(surface);
    layer.attach_new_buffer();
    layer.commit();
    f.double_roundtrip(id);
}

// What has the keyboard: "window", a layer surface's namespace, or "nothing".
fn focus(f: &mut Fixture) -> String {
    f.niri_state().update_keyboard_focus();
    let niri = f.niri();
    match &niri.keyboard_focus {
        KeyboardFocus::Layout { surface: Some(_) } => "window".to_owned(),
        KeyboardFocus::LayerShell { surface } => niri
            .mapped_layer_surfaces
            .keys()
            .find(|layer| layer.wl_surface() == surface)
            .map(|layer| layer.namespace().to_owned())
            .unwrap_or_else(|| "unknown layer".to_owned()),
        _ => "nothing".to_owned(),
    }
}

// What a click elsewhere does to on-demand focus (on_pointer_button).
fn click_elsewhere(f: &mut Fixture) {
    f.niri().focus_layer_surface_if_on_demand(None);
    f.niri_state().update_keyboard_focus();
}

#[test]
fn switch_from_none_to_on_demand_takes_focus() {
    let (mut f, id) = set_up();
    let card = map_layer(&mut f, id, "card", (400, 200), KeyboardInteractivity::None);
    assert_eq!(focus(&mut f), "window");

    set_kb(&mut f, id, &card, KeyboardInteractivity::OnDemand);
    assert_eq!(focus(&mut f), "card");

    // Back to None gives the keyboard back to the window, without a remap.
    set_kb(&mut f, id, &card, KeyboardInteractivity::None);
    assert_eq!(focus(&mut f), "window");
}

#[test]
fn commits_of_an_on_demand_surface_dont_take_focus_back() {
    let (mut f, id) = set_up();
    let card = map_layer(
        &mut f,
        id,
        "card",
        (400, 200),
        KeyboardInteractivity::OnDemand,
    );
    // Newly mapped on-demand: focused, as before.
    assert_eq!(focus(&mut f), "card");

    click_elsewhere(&mut f);
    assert_eq!(focus(&mut f), "window");

    // New frames, a resize and setting OnDemand again (no switch) leave the focus where it is.
    redraw(&mut f, id, &card);
    resize(&mut f, id, &card, (400, 300), None);
    set_kb(&mut f, id, &card, KeyboardInteractivity::OnDemand);
    assert_eq!(focus(&mut f), "window");

    // Through None and back takes it again.
    set_kb(&mut f, id, &card, KeyboardInteractivity::None);
    set_kb(&mut f, id, &card, KeyboardInteractivity::OnDemand);
    assert_eq!(focus(&mut f), "card");
}

#[test]
fn switch_from_exclusive_doesnt_count() {
    let (mut f, id) = set_up();
    let card = map_layer(
        &mut f,
        id,
        "card",
        (400, 200),
        KeyboardInteractivity::Exclusive,
    );
    assert_eq!(focus(&mut f), "card");

    set_kb(&mut f, id, &card, KeyboardInteractivity::OnDemand);
    assert_eq!(focus(&mut f), "window");
}

#[test]
fn kept_tiny_surface_grows_and_takes_focus() {
    let (mut f, id) = set_up();
    let card = kept_card(&mut f, id);
    let output = f.niri_output(1).name();

    // Kept while idle: no keyboard, and with no input region it doesn't count as dismissable,
    // so Esc and clicks go where they always do.
    assert_eq!(focus(&mut f), "window");
    assert_eq!(on_escape(&mut f), "");
    assert_eq!(on_press(&mut f, 960., 600.), "");

    // Opened: grows, takes input and switches to OnDemand in one commit.
    f.client(id).set_input_region(&card, &[(0, 0, 400, 200)]);
    resize(
        &mut f,
        id,
        &card,
        (400, 200),
        Some(KeyboardInteractivity::OnDemand),
    );
    assert_eq!(focus(&mut f), "card");
    // Esc is its own; a click outside dismisses it.
    assert_eq!(on_escape(&mut f), "");
    assert_eq!(on_press(&mut f, 960., 600.), format!("card on {output}"));

    // Hidden again: None, no input region, 1×1. The keyboard goes back, Esc isn't eaten.
    f.client(id).set_input_region(&card, &[]);
    resize(&mut f, id, &card, (1, 1), Some(KeyboardInteractivity::None));
    assert_eq!(focus(&mut f), "window");
    assert_eq!(on_escape(&mut f), "");
    assert_eq!(on_press(&mut f, 960., 600.), "");

    // And the next opening takes it again.
    f.client(id).set_input_region(&card, &[(0, 0, 400, 200)]);
    resize(
        &mut f,
        id,
        &card,
        (400, 200),
        Some(KeyboardInteractivity::OnDemand),
    );
    assert_eq!(focus(&mut f), "card");
}

#[test]
fn switch_on_an_inactive_output_doesnt_take_focus() {
    // niri focuses on-demand layers only on the active output, as for a newly mapped one.
    let config = Config::parse_mem(CONFIG).unwrap();
    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));
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
    f.niri_focus_output(1);

    let wl_output = f.client(id).output("headless-2");
    let layer = f
        .client(id)
        .create_layer(Some(&wl_output), Layer::Overlay, "card");
    let card = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        anchor: Some(Anchor::Top),
        size: Some((400, 200)),
        kb_interactivity: Some(KeyboardInteractivity::None),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);
    let layer = f.client(id).layer(&card);
    layer.attach_new_buffer();
    layer.set_size(400, 200);
    layer.ack_last_and_commit();
    f.double_roundtrip(id);

    set_kb(&mut f, id, &card, KeyboardInteractivity::OnDemand);
    assert_eq!(focus(&mut f), "window");

    // Focusing that output gives it the keyboard (it asked for it while mapped).
    f.niri_focus_output(2);
    assert_eq!(focus(&mut f), "card");
}

fn format_targets(targets: Vec<crate::layer::dismiss::DismissTarget>) -> String {
    let names: Vec<_> = targets
        .iter()
        .map(|(surface, output)| format!("{} on {output}", surface.namespace()))
        .collect();
    names.join(", ")
}

fn on_press(f: &mut Fixture, x: f64, y: f64) -> String {
    format_targets(f.niri().layers_to_dismiss_on_press(Point::from((x, y))))
}

fn on_escape(f: &mut Fixture) -> String {
    f.niri_state().update_keyboard_focus();
    format_targets(f.niri().layers_to_dismiss_on_escape())
}

// Whether rendering for `target` skips the surface with the given namespace entirely.
fn skipped(f: &mut Fixture, namespace: &str, target: RenderTarget) -> bool {
    let niri = f.niri();
    niri.update_render_elements(None);
    let (_, mapped) = niri
        .mapped_layer_surfaces
        .iter()
        .find(|(layer, _)| layer.namespace() == namespace)
        .unwrap();
    mapped.is_skipped_for(target)
}

#[test]
fn tiny_blocked_out_surface_is_skipped_in_captures() {
    let (mut f, id) = set_up();
    let card = kept_card(&mut f, id);

    // A 1×1 blocked-out surface draws nothing in a screencast (no black pixel)...
    assert!(skipped(&mut f, "card", RenderTarget::Screencast));
    // ...and is drawn as usual on the output and in screenshots (block-out-from "screencast").
    assert!(!skipped(&mut f, "card", RenderTarget::Output));
    assert!(!skipped(&mut f, "card", RenderTarget::ScreenCapture));

    // Grown to its normal size it's blocked out (a black box) as always.
    resize(&mut f, id, &card, (400, 200), None);
    assert!(!skipped(&mut f, "card", RenderTarget::Screencast));

    // Shrunk again: skipped again.
    resize(&mut f, id, &card, (1, 1), None);
    assert!(skipped(&mut f, "card", RenderTarget::Screencast));
}

#[test]
fn thin_blocked_out_surface_is_still_blocked_out() {
    let (mut f, id) = set_up();
    // 1 px wide but taller: it can show something, so it stays a black strip.
    map_layer(&mut f, id, "card", (1, 100), KeyboardInteractivity::None);
    assert!(!skipped(&mut f, "card", RenderTarget::Screencast));
}

#[test]
fn tiny_surface_without_block_out_is_drawn() {
    let (mut f, id) = set_up();
    map_layer(&mut f, id, "other", (1, 1), KeyboardInteractivity::None);
    assert!(!skipped(&mut f, "other", RenderTarget::Screencast));
    assert!(!skipped(&mut f, "other", RenderTarget::Output));
}
