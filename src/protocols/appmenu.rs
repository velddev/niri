//! KDE appmenu (org_kde_kwin_appmenu): apps publish where a window's menu bar lives.
//!
//! The menu itself is a `com.canonical.dbusmenu` object on the session bus; the app sends its
//! D-Bus service name and object path for a surface. niri only stores that address on the
//! surface, and it shows up as `app_menu` in the IPC window info, so a shell can show the
//! focused window's menu in its own bar. Electron/Chromium and Qt apps use this protocol.
//!
//! The address is untrusted client input that a shell hands to its D-Bus menu reader, so only
//! syntactically valid D-Bus names and object paths are kept (no whitespace, newlines or other
//! bytes that could break out of a line-based protocol), and sandboxed clients (security context)
//! don't see the global at all.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use smithay::reexports::wayland_server::backend::ClientId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::wayland::compositor::with_states;

use super::raw::kde_appmenu::server::{org_kde_kwin_appmenu, org_kde_kwin_appmenu_manager};
use org_kde_kwin_appmenu::OrgKdeKwinAppmenu;
use org_kde_kwin_appmenu_manager::OrgKdeKwinAppmenuManager;

const VERSION: u32 = 2;

pub struct AppmenuManagerState;

pub struct AppmenuGlobalData {
    filter: Arc<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

#[derive(Default)]
struct AppMenuUserData(Mutex<Option<niri_ipc::AppMenu>>);

/// The menu address an app published for this surface, if any.
/// Counts app menu changes on any surface, so observers can tell when to look again.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The number of app menu changes so far (wrapping).
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

pub fn app_menu(surface: &WlSurface) -> Option<niri_ipc::AppMenu> {
    with_states(surface, |states| {
        states
            .data_map
            .get::<AppMenuUserData>()
            .and_then(|data| data.0.lock().unwrap().clone())
    })
}

fn set_app_menu(surface: &WlSurface, menu: Option<niri_ipc::AppMenu>) {
    GENERATION.fetch_add(1, Ordering::Relaxed);
    if !surface.is_alive() {
        return;
    }
    with_states(surface, |states| {
        let data = states
            .data_map
            .get_or_insert_threadsafe(AppMenuUserData::default);
        *data.0.lock().unwrap() = menu;
    });
}

/// Whether `name` is a valid D-Bus bus name: a unique name (`:1.42`) or a well-known one
/// (`org.example.App`), per the D-Bus specification.
pub fn is_valid_bus_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 255 {
        return false;
    }
    let (unique, rest) = match name.strip_prefix(':') {
        Some(rest) => (true, rest),
        None => (false, name),
    };
    let mut elements = 0;
    for element in rest.split('.') {
        let Some(&first) = element.as_bytes().first() else {
            return false;
        };
        // Only unique names may have elements that start with a digit.
        if !unique && first.is_ascii_digit() {
            return false;
        }
        if !element
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return false;
        }
        elements += 1;
    }
    elements >= 2
}

/// Whether `path` is a valid D-Bus object path (`/`, `/com/canonical/menu/1A`), per the D-Bus
/// specification.
pub fn is_valid_object_path(path: &str) -> bool {
    if path == "/" {
        return true;
    }
    let Some(rest) = path.strip_prefix('/') else {
        return false;
    };
    rest.split('/').all(|element| {
        !element.is_empty()
            && element
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    })
}

/// The address to store for a `set_address` request, or `None` if it isn't a valid D-Bus
/// address.
fn validated_address(service_name: String, object_path: String) -> Option<niri_ipc::AppMenu> {
    if is_valid_bus_name(&service_name) && is_valid_object_path(&object_path) {
        Some(niri_ipc::AppMenu {
            service_name,
            object_path,
        })
    } else {
        debug!("ignoring invalid appmenu address: {service_name:?} {object_path:?}");
        None
    }
}

impl AppmenuManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<OrgKdeKwinAppmenuManager, AppmenuGlobalData>,
        D: Dispatch<OrgKdeKwinAppmenuManager, ()>,
        D: Dispatch<OrgKdeKwinAppmenu, WlSurface>,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let global_data = AppmenuGlobalData {
            filter: Arc::new(filter),
        };
        display.create_global::<D, OrgKdeKwinAppmenuManager, _>(VERSION, global_data);
        Self
    }
}

impl<D> GlobalDispatch<OrgKdeKwinAppmenuManager, AppmenuGlobalData, D> for AppmenuManagerState
where
    D: GlobalDispatch<OrgKdeKwinAppmenuManager, AppmenuGlobalData>,
    D: Dispatch<OrgKdeKwinAppmenuManager, ()>,
    D: Dispatch<OrgKdeKwinAppmenu, WlSurface>,
    D: 'static,
{
    fn bind(
        _state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        manager: New<OrgKdeKwinAppmenuManager>,
        _global_data: &AppmenuGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(manager, ());
    }

    fn can_view(client: Client, global_data: &AppmenuGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<OrgKdeKwinAppmenuManager, (), D> for AppmenuManagerState
where
    D: Dispatch<OrgKdeKwinAppmenuManager, ()>,
    D: Dispatch<OrgKdeKwinAppmenu, WlSurface>,
    D: 'static,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _manager: &OrgKdeKwinAppmenuManager,
        request: org_kde_kwin_appmenu_manager::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            org_kde_kwin_appmenu_manager::Request::Create { id, surface } => {
                data_init.init(id, surface);
            }
            org_kde_kwin_appmenu_manager::Request::Release => (),
        }
    }
}

impl<D> Dispatch<OrgKdeKwinAppmenu, WlSurface, D> for AppmenuManagerState
where
    D: Dispatch<OrgKdeKwinAppmenu, WlSurface>,
    D: 'static,
{
    fn request(
        _state: &mut D,
        _client: &Client,
        _appmenu: &OrgKdeKwinAppmenu,
        request: org_kde_kwin_appmenu::Request,
        surface: &WlSurface,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        match request {
            org_kde_kwin_appmenu::Request::SetAddress {
                service_name,
                object_path,
            } => set_app_menu(surface, validated_address(service_name, object_path)),
            org_kde_kwin_appmenu::Request::Release => (),
        }
    }

    fn destroyed(
        _state: &mut D,
        _client: ClientId,
        _appmenu: &OrgKdeKwinAppmenu,
        surface: &WlSurface,
    ) {
        set_app_menu(surface, None);
    }
}

#[macro_export]
macro_rules! delegate_appmenu {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty: ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            $crate::protocols::raw::kde_appmenu::server::org_kde_kwin_appmenu_manager::OrgKdeKwinAppmenuManager: $crate::protocols::appmenu::AppmenuGlobalData
        ] => $crate::protocols::appmenu::AppmenuManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            $crate::protocols::raw::kde_appmenu::server::org_kde_kwin_appmenu_manager::OrgKdeKwinAppmenuManager: ()
        ] => $crate::protocols::appmenu::AppmenuManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            $crate::protocols::raw::kde_appmenu::server::org_kde_kwin_appmenu::OrgKdeKwinAppmenu: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface
        ] => $crate::protocols::appmenu::AppmenuManagerState);
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bus_names() {
        for name in [
            ":1.42",
            ":1.0",
            ":0.1",
            ":1.2-3",
            "org.example.App",
            "org.kde.StatusNotifierItem-123-1",
            "com.slack.Slack",
            "a.b",
            "_x.-y",
            "org.example.App_2",
        ] {
            assert!(is_valid_bus_name(name), "{name:?} should be valid");
        }
        let long = format!("a.{}", "b".repeat(253));
        assert!(is_valid_bus_name(&long));
    }

    #[test]
    fn bad_bus_names() {
        let too_long = format!("a.{}", "b".repeat(254));
        for name in [
            "",
            ":",
            ":1",
            "a",
            "org",
            ".org.example",
            "org.example.",
            "org..example",
            "org.1example",
            "1org.example",
            "org.exam ple",
            "org.example App",
            ":1.42 /MenuBar/1",
            "org.example\nclick 1",
            "a\nclick 1\nwatch addr :1.2",
            "org.example\r",
            "org.example\t",
            "org.example\0",
            "org.exämple",
            "org/example",
            "::1.2",
            too_long.as_str(),
        ] {
            assert!(!is_valid_bus_name(name), "{name:?} should be invalid");
        }
    }

    #[test]
    fn object_paths() {
        for path in [
            "/",
            "/MenuBar/1",
            "/com/canonical/menu/1A2B3C",
            "/org/ayatana/bamf/window/12",
            "/_",
            "/a_b/C9",
        ] {
            assert!(is_valid_object_path(path), "{path:?} should be valid");
        }
    }

    #[test]
    fn bad_object_paths() {
        for path in [
            "",
            "MenuBar/1",
            "//",
            "/MenuBar/",
            "/MenuBar//1",
            "/Menu Bar",
            "/MenuBar/1 extra",
            "/p\nclick 1",
            "/p\r",
            "/a-b",
            "/a.b",
            "/ä",
        ] {
            assert!(!is_valid_object_path(path), "{path:?} should be invalid");
        }
    }

    #[test]
    fn set_address_keeps_only_valid_addresses() {
        assert_eq!(
            validated_address(":1.42".into(), "/MenuBar/1".into()),
            Some(niri_ipc::AppMenu {
                service_name: ":1.42".into(),
                object_path: "/MenuBar/1".into(),
            })
        );
        // A newline in the service name would inject commands into a line-based menu reader.
        assert_eq!(validated_address("a\nclick 1".into(), "/p".into()), None);
        assert_eq!(
            validated_address(":1.42".into(), "/p\nclick 1".into()),
            None
        );
        assert_eq!(validated_address("".into(), "".into()), None);
    }
}
