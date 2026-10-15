use crate::error::AuthError;

const MIN_PASSWORD_LENGTH: usize = 8;
const MAX_PASSWORD_LENGTH: usize = 128;
const MIN_USERNAME_LENGTH: usize = 3;
const MAX_USERNAME_LENGTH: usize = 32;

/// Common weak passwords rejected during validation.
const WEAK_PASSWORDS: &[&str] = &["password", "12345678", "123456789", "qwertyui", "abcdefgh"];

/// Validate password strength.
///
/// Rules:
/// - Length: 8-128 characters
/// - Not in the weak password blacklist (case-insensitive)
pub fn validate_password(password: &str) -> Result<(), AuthError> {
    if password.len() < MIN_PASSWORD_LENGTH {
        return Err(AuthError::WeakPassword(format!(
            "Password must be at least {MIN_PASSWORD_LENGTH} characters"
        )));
    }
    if password.len() > MAX_PASSWORD_LENGTH {
        return Err(AuthError::WeakPassword(format!(
            "Password must not exceed {MAX_PASSWORD_LENGTH} characters"
        )));
    }
    let lower = password.to_lowercase();
    if WEAK_PASSWORDS.contains(&lower.as_str()) {
        return Err(AuthError::WeakPassword("Password is too common".into()));
    }
    Ok(())
}

/// Validate username format.
///
/// Rules:
/// - Length: 3-32 characters
/// - Allowed characters: `[a-zA-Z0-9_-]`
/// - Must not start or end with `-` or `_`
pub fn validate_username(username: &str) -> Result<(), AuthError> {
    if username.len() < MIN_USERNAME_LENGTH {
        return Err(AuthError::InvalidUsername(format!(
            "Username must be at least {MIN_USERNAME_LENGTH} characters"
        )));
    }
    if username.len() > MAX_USERNAME_LENGTH {
        return Err(AuthError::InvalidUsername(format!(
            "Username must not exceed {MAX_USERNAME_LENGTH} characters"
        )));
    }
    if !username
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(AuthError::InvalidUsername(
            "Username may only contain letters, digits, underscores, and hyphens".into(),
        ));
    }
    // Safe to index: length >= 3, all ASCII
    let first = username.as_bytes()[0];
    let last = username.as_bytes()[username.len() - 1];
    if matches!(first, b'-' | b'_') || matches!(last, b'-' | b'_') {
        return Err(AuthError::InvalidUsername(
            "Username must not start or end with a hyphen or underscore".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_passwords_include_length_boundaries() {
        let max = "a".repeat(128);
        for password in ["StrongP@ss1", "StrongP@ss123", "abcDEF12", &max] {
            assert!(validate_password(password).is_ok(), "rejected {password:?}");
        }
    }

    #[test]
    fn password_length_and_weak_passwords_are_rejected() {
        let long = "a".repeat(129);
        for password in ["short", &long, "PASSWORD", "Password"]
            .into_iter()
            .chain(WEAK_PASSWORDS.iter().copied())
        {
            assert!(
                matches!(validate_password(password), Err(AuthError::WeakPassword(_))),
                "accepted {password:?}"
            );
        }
    }

    #[test]
    fn valid_usernames_include_length_boundaries_and_allowed_characters() {
        let max = "a".repeat(32);
        for username in [
            "test_user-1", "admin123", "abc123", "TestUser", "abc", "test-user", "test_user", &max,
        ] {
            assert!(validate_username(username).is_ok(), "rejected {username:?}");
        }
    }

    #[test]
    fn invalid_usernames_cover_length_characters_and_edge_separators() {
        let long = "a".repeat(33);
        let longer = "a".repeat(100);
        for username in [
            "ab", "a", "", &long, &longer, "test@user", "test user", "test.user", "test/user",
            "-test", "_test", "test-", "test_",
        ] {
            assert!(
                matches!(validate_username(username), Err(AuthError::InvalidUsername(_))),
                "accepted {username:?}"
            );
        }
    }
}
