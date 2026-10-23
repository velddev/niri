//! KDE appmenu (org_kde_kwin_appmenu): apps publish where a window's menu bar lives.
//!
//! The menu itself is a `com.canonical.dbusmenu` object on the session bus; the app sends its
//! D-Bus service name and object path for a surface. niri only stores that address on the
//! surface, and it shows up as `app_menu` in the IPC window info, so a shell can show the
//! focused window's menu in its own bar. Electron/Chromium and Qt apps use this protocol.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

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

impl AppmenuManagerState {
    pub fn new<D>(display: &DisplayHandle) -> Self
    where
        D: GlobalDispatch<OrgKdeKwinAppmenuManager, ()>,
        D: Dispatch<OrgKdeKwinAppmenuManager, ()>,
        D: Dispatch<OrgKdeKwinAppmenu, WlSurface>,
        D: 'static,
    {
        display.create_global::<D, OrgKdeKwinAppmenuManager, _>(VERSION, ());
        Self
    }
}

impl<D> GlobalDispatch<OrgKdeKwinAppmenuManager, (), D> for AppmenuManagerState
where
    D: GlobalDispatch<OrgKdeKwinAppmenuManager, ()>,
    D: Dispatch<OrgKdeKwinAppmenuManager, ()>,
    D: Dispatch<OrgKdeKwinAppmenu, WlSurface>,
    D: 'static,
{
    fn bind(
        _state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        manager: New<OrgKdeKwinAppmenuManager>,
        _global_data: &(),
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(manager, ());
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
            } => set_app_menu(
                surface,
                Some(niri_ipc::AppMenu {
                    service_name,
                    object_path,
                }),
            ),
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
            $crate::protocols::raw::kde_appmenu::server::org_kde_kwin_appmenu_manager::OrgKdeKwinAppmenuManager: ()
        ] => $crate::protocols::appmenu::AppmenuManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            $crate::protocols::raw::kde_appmenu::server::org_kde_kwin_appmenu_manager::OrgKdeKwinAppmenuManager: ()
        ] => $crate::protocols::appmenu::AppmenuManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            $crate::protocols::raw::kde_appmenu::server::org_kde_kwin_appmenu::OrgKdeKwinAppmenu: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface
        ] => $crate::protocols::appmenu::AppmenuManagerState);
    };
}
