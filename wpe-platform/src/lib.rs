#![cfg_attr(docsrs, feature(doc_cfg))]

pub use auto::*;
pub use buffer_dma_buf::*;
pub use ffi;

#[allow(warnings)]
mod auto;
mod buffer_dma_buf;
