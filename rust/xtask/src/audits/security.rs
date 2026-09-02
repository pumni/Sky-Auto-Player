use crate::Result;
use std::path::Path;

/// The detailed scanner remains implemented in `checks` during the parity
/// window. This module is the canonical audit boundary used by static checks.
pub(crate) fn run(root: &Path) -> Result<()> {
    crate::checks::security(root)
}
