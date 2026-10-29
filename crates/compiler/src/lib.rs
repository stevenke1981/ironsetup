//! Strict, source-located compiler for IronSetup's documented ISS subset.
//!
//! Unknown syntax is an error. This crate never invokes an external compiler,
//! evaluates Pascal, downloads sources, or executes preprocessor commands.
mod compile;
mod parse;
mod preprocess;

use ironsetup_core::{Diagnostic, Package};
use std::{collections::BTreeMap, path::PathBuf};

pub use compile::compile_script;
pub use preprocess::preprocess;

#[derive(Clone, Debug, Default)]
pub struct CompileOptions {
    /// Literal preprocessor definitions. Command-line definitions take priority.
    pub defines: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct CompiledScript {
    pub package: Package,
    pub output_dir: PathBuf,
    pub output_basename: String,
    pub warnings: Vec<Diagnostic>,
}
