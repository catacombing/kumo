#![allow(clippy::new_ret_no_self, clippy::too_many_arguments)]

use glib::object::IsA;
use glib::translate::*;

use crate::{BufferDMABuf, View};

pub trait BufferDMABufExtManual: IsA<BufferDMABuf> + 'static {
    #[doc(alias = "wpe_buffer_dma_buf_new")]
    fn new(
        view: &impl IsA<View>,
        width: i32,
        height: i32,
        format: u32,
        n_planes: u32,
        fds: i32,
        offsets: u32,
        strides: u32,
        modifier: u64,
    ) -> BufferDMABuf;
}

impl<O: IsA<BufferDMABuf>> BufferDMABufExtManual for O {
    fn new(
        view: &impl IsA<View>,
        width: i32,
        height: i32,
        format: u32,
        n_planes: u32,
        mut fds: i32,
        mut offsets: u32,
        mut strides: u32,
        modifier: u64,
    ) -> BufferDMABuf {
        unsafe {
            from_glib_full(ffi::wpe_buffer_dma_buf_new(
                view.as_ref().to_glib_none().0,
                width,
                height,
                format,
                n_planes,
                &mut fds,
                &mut offsets,
                &mut strides,
                modifier,
            ))
        }
    }
}
