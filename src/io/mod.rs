mod buf;
mod read_utils;
mod write_utils;

pub mod traits;

// Re-export everything so that you can access all the type from `crate::io::*`
pub use buf::*;
pub use read_utils::*;
pub use traits::*;
pub use write_utils::*;
