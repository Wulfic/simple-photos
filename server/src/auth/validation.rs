//! Input validation helpers for authentication.

use crate::error::AppError;

/// Maximum password length (in **characters**) to prevent DoS via bcrypt.
///
/// bcrypt silently truncates at 72 **bytes**, so multi-byte UTF-8 passwords
/// above 72 bytes will have their tail ignored during hashing. We cap at
/// 128 chars as a reasonable UX bound; callers should be aware of the
/// 72-byte bcrypt ceiling for high-entropy non-ASCII passwords.
pub const MAX_PASSWORD_LENGTH: usize = 128;

/// Username validation: alphanumeric + underscore, 3–50 chars
pub fn validate_username(username: &str) -> Result<(), AppError> {
    if username.len() < 3 || username.len() > 50 {
        return Err(AppError::BadRequest(
            "Username must be between 3 and 50 characters".into(),
        ));
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(AppError::BadRequest(
            "Username may only contain letters, numbers, and underscores".into(),
        ));
    }
    Ok(())
}

/// Password validation: length + complexity
pub fn validate_password(password: &str) -> Result<(), AppError> {
    if password.len() < 8 {
        return Err(AppError::BadRequest(
            "Password must be at least 8 characters".into(),
        ));
    }
    if password.len() > MAX_PASSWORD_LENGTH {
        return Err(AppError::BadRequest(format!(
            "Password must not exceed {} characters",
            MAX_PASSWORD_LENGTH
        )));
    }
    let has_upper = password.chars().any(|c| c.is_ascii_uppercase());
    let has_lower = password.chars().any(|c| c.is_ascii_lowercase());
    let has_digit = password.chars().any(|c| c.is_ascii_digit());
    if !has_upper || !has_lower || !has_digit {
        return Err(AppError::BadRequest(
            "Password must contain at least one uppercase letter, one lowercase letter, and one digit".into(),
        ));
    }
    Ok(())
}
