//! Stable cross-layer identifiers derived from display data.

/// Format a canonical VRS team signature from an already ordered roster.
///
/// Ordering is owned by the caller: `World` retains its legacy byte-order
/// roster sort and VRS sorts raw asset rosters before formatting. Keeping this
/// function formatting-only avoids a second, conflicting sort contract.
pub fn team_signature(team_name: &str, roster: &[String]) -> String {
    format!("{team_name}|{}", roster.join(","))
}

#[cfg(test)]
mod tests {
    use super::team_signature;

    #[test]
    fn signature_preserves_caller_order() {
        assert_eq!(
            team_signature("Vitality", &["apEX".into(), "ZywOo".into()]),
            "Vitality|apEX,ZywOo"
        );
    }
}
