//! The observers of the layout (foreign-toplevel, ext-workspace, IPC event stream) are skipped on
//! event loop iterations where nothing they look at changed.
//!
//! In tests, every skipped observer runs anyway and panics if it finds a change (see
//! `debug_observer_gate()`), so these tests (and every other test using the fixture) check that
//! the gate never goes stale. These tests also check that it does skip on idle iterations.

use niri_config::Config;
use niri_ipc::{Action, WorkspaceReferenceArg};
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_shell_v1::Layer;
use smithay::reexports::wayland_protocols_wlr::layer_shell::v1::client::zwlr_layer_surface_v1::KeyboardInteractivity;
use wayland_client::protocol::wl_surface::WlSurface;

use super::client::{ClientId, LayerConfigureProps};
use super::*;

fn open_window(f: &mut Fixture, id: ClientId, title: &str) -> WlSurface {
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.set_title(title);
    window.commit();
    f.roundtrip(id);

    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.ack_last_and_commit();
    f.double_roundtrip(id);

    surface
}

fn set_up() -> (Fixture, ClientId, WlSurface, WlSurface) {
    let mut f = Fixture::with_config(Config::default());
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1280, 720));

    let id = f.add_client();
    let a = open_window(&mut f, id, "a");
    let b = open_window(&mut f, id, "b");
    settle(&mut f, id);

    (f, id, a, b)
}

/// Lets the windows ack everything and finishes animations.
fn settle(f: &mut Fixture, id: ClientId) {
    for _ in 0..3 {
        f.niri_complete_animations();
        f.double_roundtrip(id);
        let surfaces: Vec<_> = f
            .client(id)
            .state
            .windows
            .iter()
            .map(|w| w.surface.clone())
            .collect();
        for surface in surfaces {
            let window = f.client(id).window(&surface);
            if window.recent_configures().next().is_some() {
                window.ack_last_and_commit();
            }
        }
        f.double_roundtrip(id);
    }
    f.niri_complete_animations();
    f.double_roundtrip(id);
}

fn do_action(f: &mut Fixture, id: ClientId, action: Action) {
    f.niri_state().do_action(action.into(), false);
    f.double_roundtrip(id);
    settle(f, id);
}

#[test]
fn idle_iterations_skip_observers() {
    let (mut f, id, _a, _b) = set_up();

    let generation = f.niri().observer_generation();
    for _ in 0..10 {
        f.double_roundtrip(id);
        f.niri_complete_animations();
    }
    assert_eq!(
        f.niri().observer_generation(),
        generation,
        "idle event loop iterations must not count as changes"
    );
}

#[test]
fn observers_follow_toplevel_changes() {
    let (mut f, id, a, b) = set_up();

    // Title changes without a commit.
    f.client(id).window(&a).set_title("a, renamed");
    f.double_roundtrip(id);

    f.client(id).window(&b).set_title("b, renamed");
    f.client(id).window(&b).commit();
    f.double_roundtrip(id);

    // Fullscreen and maximize go through configure, ack and commit.
    f.client(id).window(&a).set_fullscreen(None);
    f.double_roundtrip(id);
    settle(&mut f, id);
    f.client(id).window(&a).unset_fullscreen();
    f.double_roundtrip(id);
    settle(&mut f, id);
    f.client(id).window(&b).set_maximized();
    f.double_roundtrip(id);
    settle(&mut f, id);

    // Closing a window.
    f.client(id).window(&b).attach_null();
    f.client(id).window(&b).commit();
    f.double_roundtrip(id);
    settle(&mut f, id);

    // A new one.
    let _c = open_window(&mut f, id, "c");
    settle(&mut f, id);
}

#[test]
fn observers_follow_layout_changes() {
    let (mut f, id, _a, _b) = set_up();

    do_action(&mut f, id, Action::FocusColumnLeft {});
    do_action(&mut f, id, Action::ToggleWindowFloating { id: None });
    do_action(&mut f, id, Action::ToggleWindowFloating { id: None });
    do_action(
        &mut f,
        id,
        Action::MoveWindowToWorkspaceDown { focus: false },
    );
    do_action(&mut f, id, Action::FocusWorkspaceDown {});
    do_action(
        &mut f,
        id,
        Action::FocusWorkspace {
            reference: WorkspaceReferenceArg::Index(1),
        },
    );
    do_action(
        &mut f,
        id,
        Action::SetWorkspaceName {
            name: String::from("named"),
            workspace: None,
        },
    );
    do_action(&mut f, id, Action::ConsumeOrExpelWindowLeft { id: None });
    do_action(&mut f, id, Action::ToggleWindowUrgent { id: 1 });
    do_action(&mut f, id, Action::MoveColumnToMonitorRight {});
    do_action(&mut f, id, Action::FocusMonitorRight {});
    do_action(&mut f, id, Action::ToggleOverview {});
    do_action(&mut f, id, Action::ToggleOverview {});
    do_action(&mut f, id, Action::SwitchPresetColumnWidth {});
    do_action(&mut f, id, Action::MaximizeColumn {});
}

#[test]
fn observers_follow_focus_and_outputs() {
    let (mut f, id, _a, _b) = set_up();

    f.niri_focus_output(2);
    f.double_roundtrip(id);
    f.niri_focus_output(1);
    f.double_roundtrip(id);

    // A layer surface taking keyboard focus unfocuses the windows.
    let layer = f.client(id).create_layer(None, Layer::Top, "");
    let surface = layer.surface.clone();
    layer.set_configure_props(LayerConfigureProps {
        size: Some((100, 100)),
        kb_interactivity: Some(KeyboardInteractivity::Exclusive),
        ..Default::default()
    });
    layer.commit();
    f.roundtrip(id);
    let layer = f.client(id).layer(&surface);
    layer.attach_new_buffer();
    layer.ack_last_and_commit();
    f.double_roundtrip(id);
    settle(&mut f, id);

    let layer = f.client(id).layer(&surface);
    layer.attach_null();
    layer.commit();
    f.double_roundtrip(id);
    settle(&mut f, id);

    // Removing an output moves its workspaces.
    let output = f.niri_output(2);
    f.niri().remove_output(&output);
    f.double_roundtrip(id);
    settle(&mut f, id);
}
