use std::num::NonZeroU32;

use crate::BuildError;

/// Maximum explicitly issued calls, including the initial request.
/// This does not bound attempts made internally by the caller's SDK.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatchBudget(NonZeroU32);

impl BatchBudget {
    pub fn new(max_calls: u32) -> Result<Self, BuildError> {
        NonZeroU32::new(max_calls)
            .map(Self)
            .ok_or(BuildError::OperationConstraint {
                code: "zero_batch_budget",
            })
    }

    pub fn max_calls(self) -> u32 {
        self.0.get()
    }
}
