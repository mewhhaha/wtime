//! Optional branch metadata inspection. Invalid hints never invalidate a module
//! or change its instructions. Source: branch-hinting 23ce0bb32bbb690ce08877154e3879af060fb1d4.
use crate::{DecodedModule, ImportKind, Reader};
use wtime_types::{Error, ErrorKind, Result};
/// Predicted branch direction recorded in a `metadata.code.branch_hint` custom section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BranchLikelihood {
    /// Branch is hinted as unlikely to be taken (`0`).
    Unlikely,
    /// Branch is hinted as likely to be taken (`1`).
    Likely,
}
/// A validated branch hint targeting an `if` or `br_if` instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BranchHint {
    /// Function index in the module's function index space.
    pub function: u32,
    /// Byte offset of the target instruction relative to the start of the function body.
    pub instruction_offset: u32,
    /// Absolute byte offset of the target instruction within the module binary.
    pub absolute_offset: usize,
    /// Hinted branch likelihood.
    pub likelihood: BranchLikelihood,
}
/// Result of inspecting optional `metadata.code.branch_hint` custom sections.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BranchHints {
    /// Successfully parsed branch hints.
    pub hints: Vec<BranchHint>,
    /// Number of malformed, misplaced, or duplicate branch-hint sections ignored.
    pub ignored_sections: usize,
    /// Non-fatal diagnostic messages explaining why sections were ignored.
    pub diagnostics: Vec<String>,
}
fn invalid(message: &str) -> Error {
    Error::new(ErrorKind::Decode, message)
}
fn parse(module: &DecodedModule, bytes: &[u8]) -> Result<Vec<BranchHint>> {
    let mut reader = Reader::new(bytes);
    let count = reader.u32()? as usize;
    if count > reader.remaining() {
        return Err(invalid("hint function count exceeds payload"));
    }
    let imports = module
        .imports
        .iter()
        .filter(|i| matches!(i.kind, ImportKind::Func(_) | ImportKind::ExactFunc(_)))
        .count();
    let mut last_function = None;
    let mut hints = Vec::new();
    for _ in 0..count {
        let function = reader.u32()?;
        if last_function.is_some_and(|p| p >= function) {
            return Err(invalid("hint functions must increase"));
        }
        last_function = Some(function);
        let local = (function as usize)
            .checked_sub(imports)
            .ok_or_else(|| invalid("hint targets imported function"))?;
        let body = module
            .code
            .get(local)
            .ok_or_else(|| invalid("hint function out of bounds"))?;
        let count = reader.u32()? as usize;
        if count > reader.remaining() {
            return Err(invalid("hint count exceeds payload"));
        }
        let mut last_offset = None;
        for _ in 0..count {
            let offset = reader.u32()?;
            if last_offset.is_some_and(|p| p >= offset) {
                return Err(invalid("hint offsets must increase"));
            }
            last_offset = Some(offset);
            if reader.u32()? != 1 {
                return Err(invalid("unsupported branch hint payload size"));
            }
            let likelihood = match reader.u32()? {
                0 => BranchLikelihood::Unlikely,
                1 => BranchLikelihood::Likely,
                _ => return Err(invalid("unknown branch hint category")),
            };
            let absolute = body
                .span
                .start
                .checked_add(offset as usize)
                .ok_or_else(|| invalid("hint offset overflow"))?;
            if !body
                .instructions
                .iter()
                .zip(&body.offsets)
                .any(|(i, &start)| start as usize == absolute && matches!(i.opcode, 0x04 | 0x0d))
            {
                return Err(invalid("hint does not target if or br_if"));
            }
            hints.push(BranchHint {
                function,
                instruction_offset: offset,
                absolute_offset: absolute,
                likelihood,
            });
        }
    }
    reader.finish()?;
    Ok(hints)
}
/// Inspect optional hints without affecting binary decoding or validation.
pub fn inspect_branch_hints(module: &DecodedModule) -> BranchHints {
    let sections = module
        .customs
        .iter()
        .filter(|s| s.name == "metadata.code.branch_hint")
        .collect::<Vec<_>>();
    if sections.len() > 1 {
        return BranchHints {
            hints: vec![],
            ignored_sections: sections.len(),
            diagnostics: vec!["duplicate branch hint sections ignored".into()],
        };
    }
    let mut result = BranchHints::default();
    for section in sections {
        let parsed = if module
            .code
            .first()
            .is_some_and(|b| section.data.start > b.span.start)
        {
            Err(invalid("branch hint section must precede code"))
        } else {
            parse(module, &module.bytes[section.data.start..section.data.end])
        };
        match parsed {
            Ok(hints) => result.hints.extend(hints),
            Err(e) => {
                result.ignored_sections += 1;
                result.diagnostics.push(e.to_string());
            }
        }
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    use wtime_types::{FeatureSet, ResourceLimits};
    fn module(hints: &[u8]) -> DecodedModule {
        let mut bytes = vec![0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 3, 2, 1, 0];
        let name = b"metadata.code.branch_hint";
        let mut section = vec![name.len() as u8];
        section.extend(name);
        section.extend(hints);
        bytes.extend([0, section.len() as u8]);
        bytes.extend(section);
        bytes.extend([10, 10, 1, 8, 0, 65, 0, 4, 64, 1, 11, 11]);
        crate::decode(&bytes, &FeatureSet::default(), &ResourceLimits::default()).unwrap()
    }
    #[test]
    fn t158_case_01() {
        let m = module(&[1, 0, 1, 3, 1, 1]);
        let hints = inspect_branch_hints(&m);
        assert_eq!(hints.hints.len(), 1);
        assert_eq!(hints.hints[0].likelihood, BranchLikelihood::Likely);
        assert!(hints.diagnostics.is_empty());
    }
    #[test]
    fn t158_case_02() {
        let m = module(&[1, 0, 1, 3]);
        let hints = inspect_branch_hints(&m);
        assert_eq!(hints.ignored_sections, 1);
        assert!(hints.hints.is_empty());
        assert_eq!(m.code.len(), 1);
    }
    #[test]
    fn t158_case_03() {
        let m = module(&[1, 0, 1, 3, 1, 99]);
        let hints = inspect_branch_hints(&m);
        assert_eq!(hints.ignored_sections, 1);
        assert!(hints.hints.is_empty());
    }
}
