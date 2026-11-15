//! Production wait-guard mode configuration.
//!
//! Frozen is the only supported production wait strategy. Adaptive mode is
//! retired.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WaitGuardMode {
    #[default]
    Frozen,
}

impl WaitGuardMode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "frozen" => Ok(Self::Frozen),
            "adaptive" => Err("adaptive guard mode is retired; only 'frozen' is supported".into()),
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
        assert!(err.contains("adaptive guard mode is retired"));
    }

    #[test]
    fn parse_unknown_fails() {
        let err = WaitGuardMode::parse("other").unwrap_err();
        assert!(err.contains("unknown wait guard mode"));
    }
}
