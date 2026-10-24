use wayland_client::protocol::wl_surface::WlSurface;

use super::*;
use crate::tests::client::{ClientId, ExportResult};

// One output with a renderer and a 100×100 window.
fn set_up() -> (Fixture, ClientId, WlSurface, u32) {
    let mut f = Fixture::new();
    f.niri_state().backend.headless().add_renderer().unwrap();
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

    let (_, mapped) = f.niri().layout.windows().next().unwrap();
    let window_id = u32::try_from(mapped.id().get()).unwrap();
    (f, id, surface, window_id)
}

fn capture(f: &mut Fixture, id: ClientId, window_id: u32) -> ExportResult {
    let frame = f.client(id).capture_window(window_id, (100, 100), true);
    f.double_roundtrip(id);
    frame.result()
}

#[test]
fn capture_while_unlocked() {
    let (mut f, id, _, window_id) = set_up();
    assert_eq!(capture(&mut f, id, window_id), ExportResult::Ready);
}

#[test]
fn capture_fails_while_locked() {
    let (mut f, id, _, window_id) = set_up();

    let lock = f.client(id).lock_session();
    f.double_roundtrip(id);
    assert!(f.niri().is_locked());

    assert_eq!(capture(&mut f, id, window_id), ExportResult::Failed);

    // Works again after unlocking.
    lock.unlock_and_destroy();
    f.double_roundtrip(id);
    assert!(!f.niri().is_locked());
    assert_eq!(capture(&mut f, id, window_id), ExportResult::Ready);
}

#[test]
fn copy_waiting_for_commit_fails_after_lock() {
    let (mut f, id, surface, window_id) = set_up();

    let frame = f.client(id).capture_window(window_id, (100, 100), false);
    f.double_roundtrip(id);
    assert_eq!(frame.result(), ExportResult::Pending);

    f.client(id).lock_session();
    f.double_roundtrip(id);
    assert!(f.niri().is_locked());

    // The window keeps drawing behind the lock.
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.commit();
    f.double_roundtrip(id);
    assert_eq!(frame.result(), ExportResult::Failed);
}
