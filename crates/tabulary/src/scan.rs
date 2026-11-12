//! Validated parallel Scan segments, without scheduling or I/O.
use crate::BuildError;

/// A validated zero-based segment and total. Construction never schedules work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanSegment {
    segment: u32,
    total: u32,
}
impl ScanSegment {
    pub fn new(segment: u32, total: u32) -> Result<Self, BuildError> {
        if total == 0 || total > 1_000_000 || segment >= total {
            return Err(BuildError::OperationConstraint {
                code: "invalid_scan_segment",
            });
        }
        Ok(Self { segment, total })
    }
}
