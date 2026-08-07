//! Lowers a checked script to compiled script bytes.
//!
//! # Status
//!
//! Scaffold. The target is a stack machine. Notes for the implementation:
//!
//! - Emit into an [`crate::ncs::NcsWriter`], record jump sites, and patch them
//!   with [`crate::ncs::NcsWriter::patch_u32`] once targets are known.
//! - Every jump offset is relative to the start of the jumping instruction.
//! - Globals live in their own frame set up by an entry stub that runs before
//!   `main`. Scripts with no globals still get the stub in the original
//!   compiler's output, so match that if byte equality matters.
//! - Byte-for-byte equality with `nwnnsscomp.exe` output is the acceptance
//!   test. Where a choice is arbitrary, copy what the original does rather
//!   than what is tidy. The golden corpus is the arbiter.

use crate::error::CompileError;
use crate::sema::AnalyzedScript;

/// How hard to work at shrinking the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OptimizationLevel {
    /// Emit what the tree says, instruction for instruction.
    ///
    /// This is the default because it is what the original compiler does
    /// without `-o`, and byte equality with it is the goal.
    #[default]
    None,
    /// Fold constants and drop unreachable code, as the original `-o` does.
    Basic,
}

/// Turns a checked script into compiled script bytes, header included.
///
/// # Errors
///
/// Returns an error when lowering fails or a jump cannot be resolved.
pub fn generate(
    script: &AnalyzedScript,
    optimization: OptimizationLevel,
) -> Result<Vec<u8>, CompileError> {
    let _ = (script, optimization);
    // TODO(codegen): walk the checked tree and emit the instruction stream.
    Err(CompileError::NotImplemented("code generation"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optimization_defaults_to_off() {
        assert_eq!(OptimizationLevel::default(), OptimizationLevel::None);
    }
}
