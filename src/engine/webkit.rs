use std::any::Any;
use std::ffi::{self, CString};
use std::ptr;
use std::sync::Once;

use funq::StQueueHandle;
use glutin::api::egl::EGL;
use glutin::display::{AsRawDisplay, Display, RawDisplay};
use smithay_client_toolkit::reexports::client::protocol::wl_buffer::WlBuffer;
use smithay_client_toolkit::reexports::client::{Connection, Proxy};
use wayland_backend::client::ObjectId;
use wpe_backend_fdo_sys::{
    wpe_fdo_egl_exported_image, wpe_fdo_egl_exported_image_get_egl_image,
    wpe_fdo_egl_exported_image_get_height, wpe_fdo_egl_exported_image_get_width,
    wpe_view_backend_exportable_fdo, wpe_view_backend_exportable_fdo_dispatch_frame_complete,
    wpe_view_backend_exportable_fdo_egl_client, wpe_view_backend_exportable_fdo_egl_create,
    wpe_view_backend_exportable_fdo_egl_dispatch_release_exported_image,
    wpe_view_backend_exportable_fdo_get_view_backend,
};
use wpe_webkit::{WebView, WebViewBackend, WebViewExt};

use crate::engine::{Engine, EngineId};
use crate::State;

// Once for calling FDO initialization methods.
static FDO_INIT: Once = Once::new();

/// WebKit-specific errors.
#[derive(thiserror::Error, Debug)]
pub enum WebKitError {
    #[error("backend creation failed")]
    BackendCreation,
    #[error("could not load libWPEBackend-fdo-1.0.so, make sure it is installed")]
    FdoLibInit,
    #[error("failed to initialize fdo egl backend")]
    EglInit,
}

#[funq::callbacks(State, thread_local)]
trait WebKitHandler {
    fn set_egl_image(&mut self, engine_id: EngineId, image: *mut wpe_fdo_egl_exported_image);
}

impl WebKitHandler for State {
    fn set_egl_image(&mut self, engine_id: EngineId, image: *mut wpe_fdo_egl_exported_image) {
        let wayland_queue = self.wayland_queue();

        let engine = match self.engines.get_mut(&engine_id) {
            Some(engine) => engine,
            None => return,
        };

        let webkit_engine = match engine.as_any().downcast_mut::<WebKitEngine>() {
            Some(webkit_engine) => webkit_engine,
            None => return,
        };

        // Request new image if submitted one is of incorrect size.
        unsafe {
            let width = wpe_fdo_egl_exported_image_get_width(image);
            let height = wpe_fdo_egl_exported_image_get_height(image);
            let desired_width = (webkit_engine.width as f32 * webkit_engine.scale).round() as u32;
            let desired_height = (webkit_engine.height as f32 * webkit_engine.scale).round() as u32;

            if desired_width != width || desired_height != height {
                webkit_engine.frame_done();
                wpe_view_backend_exportable_fdo_egl_dispatch_release_exported_image(
                    webkit_engine.exportable,
                    image,
                );
                return;
            }
        }

        // Update engine's WlBuffer.
        webkit_engine.import_image(&self.connection, &self.egl_display, image);

        // Offer new WlBuffer to window.
        let window_id = engine_id.window_id();
        if let Some(window) = self.windows.get_mut(&window_id) {
            window.mark_engine_dirty(&self.connection, &wayland_queue, &self.engines, engine_id);
        }
    }
}

/// WebKit browser engine.
pub struct WebKitEngine {
    id: EngineId,

    backend: WebViewBackend,
    web_view: WebView,

    exportable: *mut wpe_view_backend_exportable_fdo,
    image: *mut wpe_fdo_egl_exported_image,
    buffer: Option<WlBuffer>,

    width: u32,
    height: u32,
    scale: f32,
}

impl Drop for WebKitEngine {
    fn drop(&mut self) {
        unsafe {
            // Free EGL image.
            if !self.image.is_null() {
                wpe_view_backend_exportable_fdo_egl_dispatch_release_exported_image(
                    self.exportable,
                    self.image,
                );
            }
        }
    }
}

impl WebKitEngine {
    pub fn new(
        display: &Display,
        queue: StQueueHandle<State>,
        engine_id: EngineId,
        width: u32,
        height: u32,
    ) -> Result<Self, WebKitError> {
        // Ensure FDO is initialized.
        let mut result = Ok(());
        FDO_INIT.call_once(|| result = Self::init_fdo(display));
        result?;

        // Create web view backend.
        let (mut backend, exportable) = unsafe {
            // Create EGL FDO backend.
            let exportable = create_exportable_backend(engine_id, queue, width, height);
            let egl_backend = wpe_view_backend_exportable_fdo_get_view_backend(exportable);
            if egl_backend.is_null() {
                return Err(WebKitError::BackendCreation);
            }

            (WebViewBackend::new(egl_backend), exportable)
        };

        // TODO: Multipe `WebView`s should share the same context / backend(?).
        // TODO: WEBKIT_PROCESS_MODEL_MULTIPLE_SECONDARY_PROCESSES
        //
        // Create web view with initial blank page.
        let web_view = WebView::new(&mut backend);
        web_view.load_uri("about:blank");

        Ok(Self {
            exportable,
            web_view,
            backend,
            width,
            height,
            image: ptr::null_mut(),
            id: engine_id,
            scale: 1.0,
            buffer: Default::default(),
        })
    }

    /// Import a new EGLImage as WlBuffer.
    fn import_image(
        &mut self,
        connection: &Connection,
        egl_display: &Display,
        image: *mut wpe_fdo_egl_exported_image,
    ) {
        // Free previous image.
        if !self.image.is_null() {
            unsafe {
                wpe_view_backend_exportable_fdo_egl_dispatch_release_exported_image(
                    self.exportable,
                    self.image,
                );
            }
        }

        self.image = image;

        let RawDisplay::Egl(raw_display) = egl_display.raw_display();
        let egl = EGL.as_ref().unwrap();

        // Convert EGLImage to WlBuffer.
        let object_id = unsafe {
            let egl_image = wpe_fdo_egl_exported_image_get_egl_image(self.image);
            let raw_wl_buffer = egl.CreateWaylandBufferFromImageWL(raw_display, egl_image);
            ObjectId::from_ptr(WlBuffer::interface(), raw_wl_buffer.cast()).ok().unwrap()
        };
        self.buffer = Some(WlBuffer::from_id(connection, object_id).unwrap());
    }

    /// Initialize the WPEBackend-fdo library.
    fn init_fdo(display: &Display) -> Result<(), WebKitError> {
        unsafe {
            let RawDisplay::Egl(display) = display.raw_display();

            let backend_lib = CString::new("libWPEBackend-fdo-1.0.so").unwrap();
            if !wpe_backend_fdo_sys::wpe_loader_init(backend_lib.as_ptr()) {
                return Err(WebKitError::FdoLibInit);
            }

            if !wpe_backend_fdo_sys::wpe_fdo_initialize_for_egl_display(display as *mut _) {
                return Err(WebKitError::EglInit);
            }

            Ok(())
        }
    }
}

impl Engine for WebKitEngine {
    fn id(&self) -> EngineId {
        self.id
    }

    fn wl_buffer(&self) -> Option<&WlBuffer> {
        self.buffer.as_ref()
    }

    fn frame_done(&self) {
        unsafe {
            wpe_view_backend_exportable_fdo_dispatch_frame_complete(self.exportable);
        }
    }

    fn set_size(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;

        unsafe {
            let wpe_backend = self.backend.wpe_backend();
            wpe_backend_fdo_sys::wpe_view_backend_dispatch_set_size(wpe_backend, width, height);
        }
    }

    fn set_scale(&mut self, scale: f64) {
        // Clamp scale to WebKit's limits.
        //
        // https://webplatformforembedded.github.io/libwpe/view-backend.html#wpe_view_backend_dispatch_set_device_scale_factor
        self.scale = scale.clamp(0.05, 5.0) as f32;

        unsafe {
            let wpe_backend = self.backend.wpe_backend();
            wpe_backend_fdo_sys::wpe_view_backend_dispatch_set_device_scale_factor(
                wpe_backend,
                self.scale,
            );
        }
    }

    fn load_uri(&self, uri: &str) {
        self.web_view.load_uri(uri);
    }

    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

/// Shared state leaked to FDO backend callbacks.
struct ExportableSharedState {
    queue: StQueueHandle<State>,
    engine_id: EngineId,
}

/// Create the exportable FDO EGL backend.
unsafe fn create_exportable_backend(
    engine_id: EngineId,
    queue: StQueueHandle<State>,
    width: u32,
    height: u32,
) -> *mut wpe_view_backend_exportable_fdo {
    let client = wpe_view_backend_exportable_fdo_egl_client {
        export_fdo_egl_image: Some(on_egl_image_export),
        export_shm_buffer: None,
        export_egl_image: None,
        _wpe_reserved0: None,
        _wpe_reserved1: None,
    };

    let client = Box::into_raw(Box::new(client));
    let state = Box::into_raw(Box::new(ExportableSharedState { engine_id, queue }));
    wpe_view_backend_exportable_fdo_egl_create(client, state.cast(), width, height)
}

/// Handle EGL backend image export.
unsafe extern "C" fn on_egl_image_export(
    data: *mut ffi::c_void,
    image: *mut wpe_fdo_egl_exported_image,
) {
    let state = data as *mut ExportableSharedState;
    let state = match state.as_mut() {
        Some(state) => state,
        None => return,
    };

    state.queue.set_egl_image(state.engine_id, image);
}
