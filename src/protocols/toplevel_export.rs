//! hyprland-toplevel-export-v1: capture the contents of a single window.
//!
//! This is what Quickshell's `ScreencopyView` uses to show live window contents. Frames are
//! offered at the window's size in physical pixels. A copy with `ignore_damage` set is rendered
//! right away; otherwise it waits for the window's next commit, so live captures follow the
//! window's own frame rate instead of spinning.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use calloop::generic::Generic;
use calloop::{Interest, LoopHandle, Mode, PostAction};
use smithay::backend::allocator::dmabuf::Dmabuf;
use smithay::backend::allocator::{Buffer, Fourcc};
use smithay::backend::renderer::sync::SyncPoint;
use smithay::reexports::wayland_protocols_wlr::foreign_toplevel::v1::server::zwlr_foreign_toplevel_handle_v1::ZwlrForeignToplevelHandleV1;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_shm::Format;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Physical, Size};
use smithay::wayland::{dmabuf, shm};

use super::raw::hyprland_toplevel_export::v1::server::{
    hyprland_toplevel_export_frame_v1, hyprland_toplevel_export_manager_v1,
};
use crate::utils::get_monotonic_time;
use hyprland_toplevel_export_frame_v1::{Flags, HyprlandToplevelExportFrameV1};
use hyprland_toplevel_export_manager_v1::HyprlandToplevelExportManagerV1;

const VERSION: u32 = 2;

/// Copies waiting for their window to commit new contents.
#[derive(Default)]
pub struct ToplevelExportManagerState {
    waiting: Vec<ToplevelExport>,
}

pub struct ToplevelExportGlobalData {
    filter: Box<dyn for<'c> Fn(&'c Client) -> bool + Send + Sync>,
}

impl ToplevelExportManagerState {
    pub fn new<D, F>(display: &DisplayHandle, filter: F) -> Self
    where
        D: GlobalDispatch<HyprlandToplevelExportManagerV1, ToplevelExportGlobalData>,
        D: Dispatch<HyprlandToplevelExportManagerV1, ()>,
        D: Dispatch<HyprlandToplevelExportFrameV1, ToplevelExportFrameState>,
        D: ToplevelExportHandler,
        D: 'static,
        F: for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static,
    {
        let global_data = ToplevelExportGlobalData {
            filter: Box::new(filter),
        };
        display.create_global::<D, HyprlandToplevelExportManagerV1, _>(VERSION, global_data);

        Self::default()
    }

    pub fn push(&mut self, export: ToplevelExport) {
        self.waiting.push(export);
    }

    /// Whether any copy is waiting for this window.
    pub fn is_waiting_for(&self, surface: &WlSurface) -> bool {
        self.waiting.iter().any(|export| export.surface == *surface)
    }

    /// Take the copies waiting for this window, dropping any whose frame is gone.
    pub fn take_for(&mut self, surface: &WlSurface) -> Vec<ToplevelExport> {
        self.waiting.retain(|export| export.frame.is_alive());
        let (taken, rest) = std::mem::take(&mut self.waiting)
            .into_iter()
            .partition(|export| export.surface == *surface);
        self.waiting = rest;
        taken
    }

    /// Fail the copies waiting for a window that went away.
    pub fn remove_surface(&mut self, surface: &WlSurface) {
        drop(self.take_for(surface));
    }
}

impl<D> GlobalDispatch<HyprlandToplevelExportManagerV1, ToplevelExportGlobalData, D>
    for ToplevelExportManagerState
where
    D: GlobalDispatch<HyprlandToplevelExportManagerV1, ToplevelExportGlobalData>,
    D: Dispatch<HyprlandToplevelExportManagerV1, ()>,
    D: Dispatch<HyprlandToplevelExportFrameV1, ToplevelExportFrameState>,
    D: ToplevelExportHandler,
    D: 'static,
{
    fn bind(
        _state: &mut D,
        _dh: &DisplayHandle,
        _client: &Client,
        manager: New<HyprlandToplevelExportManagerV1>,
        _global_data: &ToplevelExportGlobalData,
        data_init: &mut DataInit<'_, D>,
    ) {
        data_init.init(manager, ());
    }

    fn can_view(client: Client, global_data: &ToplevelExportGlobalData) -> bool {
        (global_data.filter)(&client)
    }
}

impl<D> Dispatch<HyprlandToplevelExportManagerV1, (), D> for ToplevelExportManagerState
where
    D: Dispatch<HyprlandToplevelExportManagerV1, ()>,
    D: Dispatch<HyprlandToplevelExportFrameV1, ToplevelExportFrameState>,
    D: ToplevelExportHandler,
    D: 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        _manager: &HyprlandToplevelExportManagerV1,
        request: hyprland_toplevel_export_manager_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, D>,
    ) {
        let (frame, overlay_cursor, surface) = match request {
            // The uint handle is a niri window id.
            hyprland_toplevel_export_manager_v1::Request::CaptureToplevel {
                frame,
                overlay_cursor,
                handle,
            } => (
                frame,
                overlay_cursor,
                state.surface_for_window_id(u64::from(handle)),
            ),
            hyprland_toplevel_export_manager_v1::Request::CaptureToplevelWithWlrToplevelHandle {
                frame,
                overlay_cursor,
                handle,
            } => (frame, overlay_cursor, state.surface_for_wlr_handle(&handle)),
            hyprland_toplevel_export_manager_v1::Request::Destroy => return,
        };

        let Some((surface, buffer_size)) =
            surface.and_then(|s| state.export_size(&s).map(|size| (s, size)))
        else {
            trace!("toplevel export requested for a window that is gone or empty");
            let frame = data_init.init(frame, ToplevelExportFrameState::Failed);
            frame.failed();
            return;
        };

        let info = ToplevelExportInfo {
            surface,
            buffer_size,
            overlay_cursor: overlay_cursor != 0,
        };
        let frame = data_init.init(
            frame,
            ToplevelExportFrameState::Pending {
                info,
                copied: Arc::new(AtomicBool::new(false)),
            },
        );

        let (w, h) = (buffer_size.w as u32, buffer_size.h as u32);
        frame.buffer(Format::Xrgb8888, w, h, w * 4);
        if frame.version() >= 2 {
            frame.linux_dmabuf(Fourcc::Xrgb8888 as u32, w, h);
            frame.buffer_done();
        }
    }
}

/// Handler trait for hyprland-toplevel-export.
pub trait ToplevelExportHandler {
    fn toplevel_export_state(&mut self) -> &mut ToplevelExportManagerState;

    /// The toplevel surface behind a wlr foreign toplevel handle.
    fn surface_for_wlr_handle(&mut self, handle: &ZwlrForeignToplevelHandleV1)
        -> Option<WlSurface>;

    /// The toplevel surface of the window with this niri window id.
    fn surface_for_window_id(&mut self, id: u64) -> Option<WlSurface>;

    /// Size in physical pixels to capture this window at, or None if it has no contents.
    fn export_size(&mut self, surface: &WlSurface) -> Option<Size<i32, Physical>>;

    /// A copy request came in. Must render it, fail it, or push it to the state.
    fn export_frame(&mut self, export: ToplevelExport, ignore_damage: bool);
}

#[allow(missing_docs)]
#[macro_export]
macro_rules! delegate_toplevel_export {
    ($(@<$( $lt:tt $( : $clt:tt $(+ $dlt:tt )* )? ),+>)? $ty: ty) => {
        smithay::reexports::wayland_server::delegate_global_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            $crate::protocols::raw::hyprland_toplevel_export::v1::server::hyprland_toplevel_export_manager_v1::HyprlandToplevelExportManagerV1: $crate::protocols::toplevel_export::ToplevelExportGlobalData
        ] => $crate::protocols::toplevel_export::ToplevelExportManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            $crate::protocols::raw::hyprland_toplevel_export::v1::server::hyprland_toplevel_export_manager_v1::HyprlandToplevelExportManagerV1: ()
        ] => $crate::protocols::toplevel_export::ToplevelExportManagerState);

        smithay::reexports::wayland_server::delegate_dispatch!($(@< $( $lt $( : $clt $(+ $dlt )* )? ),+ >)? $ty: [
            $crate::protocols::raw::hyprland_toplevel_export::v1::server::hyprland_toplevel_export_frame_v1::HyprlandToplevelExportFrameV1: $crate::protocols::toplevel_export::ToplevelExportFrameState
        ] => $crate::protocols::toplevel_export::ToplevelExportManagerState);
    };
}

#[derive(Clone)]
pub struct ToplevelExportInfo {
    surface: WlSurface,
    buffer_size: Size<i32, Physical>,
    overlay_cursor: bool,
}

pub enum ToplevelExportFrameState {
    Failed,
    Pending {
        info: ToplevelExportInfo,
        copied: Arc<AtomicBool>,
    },
}

impl<D> Dispatch<HyprlandToplevelExportFrameV1, ToplevelExportFrameState, D>
    for ToplevelExportManagerState
where
    D: Dispatch<HyprlandToplevelExportFrameV1, ToplevelExportFrameState>,
    D: ToplevelExportHandler,
    D: 'static,
{
    fn request(
        state: &mut D,
        _client: &Client,
        frame: &HyprlandToplevelExportFrameV1,
        request: hyprland_toplevel_export_frame_v1::Request,
        data: &ToplevelExportFrameState,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, D>,
    ) {
        let hyprland_toplevel_export_frame_v1::Request::Copy {
            buffer,
            ignore_damage,
        } = request
        else {
            return;
        };

        let ToplevelExportFrameState::Pending { info, copied } = data else {
            return;
        };

        if copied.swap(true, Ordering::SeqCst) {
            frame.post_error(
                hyprland_toplevel_export_frame_v1::Error::AlreadyUsed,
                "copy was already requested",
            );
            return;
        }

        let size = info.buffer_size;
        let buffer = if let Ok(dmabuf) = dmabuf::get_dmabuf(&buffer) {
            if dmabuf.format().code == Fourcc::Xrgb8888
                && dmabuf.width() == size.w as u32
                && dmabuf.height() == size.h as u32
            {
                ExportBuffer::Dmabuf(dmabuf.clone())
            } else {
                frame.post_error(
                    hyprland_toplevel_export_frame_v1::Error::InvalidBuffer,
                    "invalid dmabuf parameters",
                );
                return;
            }
        } else if shm::with_buffer_contents(&buffer, |_, shm_len, buffer_data| {
            buffer_data.format == Format::Xrgb8888
                && buffer_data.width == size.w
                && buffer_data.height == size.h
                && buffer_data.stride == size.w * 4
                && shm_len == buffer_data.stride as usize * buffer_data.height as usize
        })
        .unwrap_or(false)
        {
            ExportBuffer::Shm(buffer)
        } else {
            frame.post_error(
                hyprland_toplevel_export_frame_v1::Error::InvalidBuffer,
                "invalid buffer",
            );
            return;
        };

        state.export_frame(
            ToplevelExport {
                info: info.clone(),
                surface: info.surface.clone(),
                frame: frame.clone(),
                buffer,
                submitted: false,
            },
            ignore_damage != 0,
        );
    }
}

/// Buffer a window is copied into.
#[derive(Clone)]
pub enum ExportBuffer {
    Dmabuf(Dmabuf),
    Shm(WlBuffer),
}

/// A copy of one window into a client buffer. Fails the frame if dropped unsubmitted.
pub struct ToplevelExport {
    info: ToplevelExportInfo,
    surface: WlSurface,
    frame: HyprlandToplevelExportFrameV1,
    buffer: ExportBuffer,
    submitted: bool,
}

impl Drop for ToplevelExport {
    fn drop(&mut self) {
        if !self.submitted && self.frame.is_alive() {
            self.frame.failed();
        }
    }
}

impl ToplevelExport {
    pub fn surface(&self) -> &WlSurface {
        &self.surface
    }

    pub fn buffer(&self) -> &ExportBuffer {
        &self.buffer
    }

    pub fn buffer_size(&self) -> Size<i32, Physical> {
        self.info.buffer_size
    }

    pub fn overlay_cursor(&self) -> bool {
        self.info.overlay_cursor
    }

    fn submit(mut self, timestamp: Duration) {
        let size = self.info.buffer_size;
        self.frame.flags(Flags::empty());
        self.frame.damage(0, 0, size.w as u32, size.h as u32);
        let tv_sec_hi = (timestamp.as_secs() >> 32) as u32;
        let tv_sec_lo = (timestamp.as_secs() & 0xFFFFFFFF) as u32;
        self.frame
            .ready(tv_sec_hi, tv_sec_lo, timestamp.subsec_nanos());
        self.submitted = true;
    }

    pub fn submit_after_sync<T>(
        self,
        sync_point: Option<SyncPoint>,
        event_loop: &LoopHandle<'_, T>,
    ) {
        let timestamp = get_monotonic_time();
        match sync_point.and_then(|s| s.export()) {
            None => self.submit(timestamp),
            Some(sync_fd) => {
                let source = Generic::new(sync_fd, Interest::READ, Mode::OneShot);
                let mut export = Some(self);
                event_loop
                    .insert_source(source, move |_, _, _| {
                        export.take().unwrap().submit(timestamp);
                        Ok(PostAction::Remove)
                    })
                    .unwrap();
            }
        }
    }
}
