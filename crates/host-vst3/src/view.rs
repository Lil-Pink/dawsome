//! A plugin's own editor (`IPlugView`) in a window of ours: opened as the format's call sequence
//! has it (the view, whether it takes our window type, its size, whether it resizes; our window;
//! its frame; attached), and closed in reverse on every way out. The window is host-common's
//! ([`Display::open_embedder`]: on Linux an XEmbed embedder, so an editor that is an XEmbed client
//! is mapped and given our focus; on Windows a window whose child the editor's window becomes);
//! the frame, through which the editor asks for another size and finds its run loop, is
//! [`PlugFrame`].
//!
//! Nothing here holds a borrow of the host's table or of the instance's editor across a call into
//! the plugin: editors ask to be resized inside `attached`, and that call reaches the frame alone.

use std::cell::Ref;
use std::ffi::c_void;

use dawsome_engine::processor::HostError;
use dawsome_host_common::window::{Display, WindowId};
#[cfg(windows)]
use vst3::Steinberg::kPlatformTypeHWND;
#[cfg(not(windows))]
use vst3::Steinberg::kPlatformTypeX11EmbedWindowID;
use vst3::Steinberg::Vst::{IEditController, IEditControllerTrait, ViewType};
use vst3::Steinberg::{kResultOk, kResultTrue, FIDString, IPlugView, IPlugViewTrait, ViewRect};
use vst3::{ComPtr, ComWrapper};

use crate::com::frame::{self, PlugFrame};

/// The window type an editor is asked to take: an X11 window it embeds into (XEmbed), or on
/// Windows an HWND.
#[cfg(not(windows))]
const PLATFORM: FIDString = kPlatformTypeX11EmbedWindowID;
#[cfg(windows)]
const PLATFORM: FIDString = kPlatformTypeHWND;

/// The size an editor that will not say its own is opened at (the CLAP host's).
const DEFAULT_SIZE: (u32, u32) = (640, 480);

/// An open editor: the view, its frame (which holds our window) and whether it was attached.
pub(crate) struct OpenView {
    view: Option<ComPtr<IPlugView>>,
    frame: ComWrapper<PlugFrame>,
    attached: bool,
}

impl OpenView {
    /// Our window it sits in.
    pub(crate) fn window(&self) -> WindowId {
        self.frame.window().id()
    }

    pub(crate) fn frame(&self) -> &ComWrapper<PlugFrame> {
        &self.frame
    }

    /// The view and its frame, for a call into the plugin made outside the borrow that keeps
    /// them.
    pub(crate) fn parts(&self) -> Option<(ComPtr<IPlugView>, ComWrapper<PlugFrame>)> {
        Some((self.view.clone()?, self.frame.clone()))
    }
}

impl Drop for OpenView {
    // S8, in this order: no resize answered from now, the view removed (when it was attached)
    // and its frame taken away, the frame's pointer to it forgotten and what it registered with
    // the frame's run loop gone, our reference let go, our window closed.
    fn drop(&mut self) {
        self.frame.close();
        if let Some(view) = self.view.take() {
            if self.attached {
                // SAFETY: the view is live (our reference); called on the main thread, once, as
                // the format's teardown orders it.
                unsafe { view.removed() };
            }
            // SAFETY: as for `removed`; from here the view holds no frame of ours.
            unsafe { view.setFrame(std::ptr::null_mut()) };
            self.frame.forget_view();
            drop(view);
        }
        self.frame.window().close();
    }
}

/// The view's size as it says it, or [`DEFAULT_SIZE`].
fn size_of(view: &ComPtr<IPlugView>) -> Option<(u32, u32)> {
    let mut rect = ViewRect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: the view is live; called on the main thread; `rect` is our out parameter.
    (unsafe { view.getSize(&mut rect) } == kResultOk).then(|| frame::size_of(&rect))
}

/// Opens `controller`'s editor in a window of ours titled `title`. `None`: it has none here (no
/// editor, not our window type, our window would not open, or the editor would not attach to
/// it); the plugin's parameter window stands in. A missing window system is the error, with the
/// reason to show, asked only once the plugin has an editor that takes our window type.
pub(crate) fn open<'a>(
    controller: &ComPtr<IEditController>,
    title: &str,
    display: impl Fn() -> Result<Ref<'a, Display>, HostError>,
) -> Result<Option<OpenView>, HostError> {
    // SAFETY: the controller is live; called on the main thread; the name is the SDK's constant.
    let raw = unsafe { controller.createView(ViewType::kEditor) };
    // SAFETY: `createView` hands its caller one reference, which this takes over.
    let Some(view) = (unsafe { ComPtr::from_raw(raw) }) else {
        return Ok(None);
    };
    // SAFETY: the view is live; `PLATFORM` is the SDK's NUL-terminated constant.
    if unsafe { view.isPlatformTypeSupported(PLATFORM) } != kResultTrue {
        return Ok(None);
    }
    // No window system: the view goes (dropped here) and the reason is shown.
    drop(display()?);
    let size = size_of(&view).unwrap_or(DEFAULT_SIZE);
    // SAFETY: as above.
    let resizable = unsafe { view.canResize() } == kResultTrue;
    let handle = {
        let display = display()?;
        let Ok(window) = display.open_embedder(title, size.0, size.1, resizable) else {
            return Ok(None);
        };
        display.handle(window)
    };
    let frame = PlugFrame::new(view.as_ptr(), handle.clone(), size, !resizable);
    let mut opened = OpenView {
        view: Some(view),
        frame,
        attached: false,
    };
    let attached = opened.view.as_ref().is_some_and(|view| {
        // SAFETY: the view is live; the frame is our object, which it keeps until `setFrame`
        // null (`OpenView`'s drop). A plugin may call the frame inside the call, and the frame
        // needs nothing borrowed here.
        unsafe { view.setFrame(PlugFrame::as_frame(&opened.frame)) };
        // SAFETY: the view is live; our window lives until after the view is removed
        // (`OpenView`'s drop); `PLATFORM` is the SDK's NUL-terminated constant.
        let result = unsafe { view.attached(handle.id() as usize as *mut c_void, PLATFORM) };
        result == kResultOk
    });
    if !attached {
        // Dropped unattached: its frame taken away, our window closed, the panel instead.
        return Ok(None);
    }
    opened.attached = true;
    // The size it settled on as it attached: our window takes it, no onSize (it chose it).
    let settled = opened.view.as_ref().and_then(size_of);
    if let Some(settled) = settled.filter(|&settled| settled != opened.frame.size()) {
        opened.frame.size_window(settled);
    }
    Ok(Some(opened))
}
