//! Production wait-guard mode configuration (Phase D, #646).
//!
//! Frozen is the single shipping production wait strategy. Adaptive mode was
//! concluded as inconclusive in Phase C and retired in Phase D.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WaitGuardMode {
    #[default]
    Frozen,
}

impl WaitGuardMode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "frozen" => Ok(Self::Frozen),
            "adaptive" => Err(
                "adaptive guard mode was retired in Phase D (#646); only 'frozen' is supported"
                    .to_string(),
            ),
            _ => Err(format!(
                "unknown wait guard mode '{value}', expected 'frozen'"
            )),
        }
    }

    #[inline]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Frozen => "frozen",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_frozen_succeeds() {
        assert_eq!(
            WaitGuardMode::parse("frozen").unwrap(),
            WaitGuardMode::Frozen
        );
        assert_eq!(
            WaitGuardMode::parse("  FROZEN  ").unwrap(),
            WaitGuardMode::Frozen
        );
        assert_eq!(WaitGuardMode::Frozen.label(), "frozen");
    }

    #[test]
    fn parse_adaptive_fails_cleanly() {
        let err = WaitGuardMode::parse("adaptive").unwrap_err();
        assert!(err.contains("retired in Phase D"));
    }

    #[test]
    fn parse_unknown_fails() {
        let err = WaitGuardMode::parse("other").unwrap_err();
        assert!(err.contains("unknown wait guard mode"));
    }
}
