pub mod bundle;
pub mod model;
pub mod paths;
pub mod selection;
pub use bundle::{
    append_package, decode_package, encode_package, read_embedded_package, read_package_file,
    stub_bytes, validate_package, write_package_file,
};
pub use model::*;
pub use paths::{checked_join, reject_symlink_ancestors, validate_relative_path};
pub use selection::evaluate_condition;

pub fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}
