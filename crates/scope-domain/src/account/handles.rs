/// Handles reserved for Scope's top-level application and marketing routes.
pub const RESERVED_HANDLES: &[&str] = &[
    "account",
    "api",
    "cli-login",
    "docs",
    "invites",
    "pricing",
    "repos",
    "sign-in",
    "sign-up",
];

pub fn is_reserved_handle(handle: &str) -> bool {
    RESERVED_HANDLES
        .iter()
        .any(|reserved| handle.eq_ignore_ascii_case(reserved))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_and_marketing_routes_are_reserved_handles() {
        for handle in [
            "account",
            "sign-in",
            "sign-up",
            "cli-login",
            "invites",
            "api",
            "docs",
            "pricing",
            "repos",
        ] {
            assert!(is_reserved_handle(handle), "{handle} must be reserved");
        }
    }

    #[test]
    fn reservation_is_case_insensitive_but_does_not_claim_derived_handles() {
        assert!(is_reserved_handle("ACCOUNT"));
        assert!(!is_reserved_handle("account-2"));
        assert!(!is_reserved_handle("adamblumoff"));
    }
}
