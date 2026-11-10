//! Android boot images with header versions 0 to 4, and the newc archives
//! their ramdisks are: read, changed and written again, by the image packer
//! on a computer and by the recovery on the phone.

mod image;
mod ramdisk;

pub use image::{BootImage, Header};
pub use ramdisk::{lz4_legacy, Compression, Entry, Ramdisk};
