#![cfg_attr(docsrs, feature(doc_cfg))]

pub use auto::builders::*;
pub use auto::traits::*;
pub use auto::*;
pub use buffer_dma_buf::*;
pub use display::*;
pub use ffi;
pub use input_method_context::*;
pub use settings::*;
pub use toplevel::*;
pub use view::*;

#[allow(warnings)]
mod auto;
mod buffer_dma_buf;
mod display;
mod input_method_context;
mod rectangle;
mod settings;
mod toplevel;
mod view;
