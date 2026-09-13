//! Fixture for the search-ranking E2E suite.
//!
//! Deliberately models the reported failure mode: the `login` definition
//! carries **no doc comment**, so its behaviour is only observable through
//! the callees in its body. The helpers it calls carry prose-rich doc
//! comments full of "email" / "password" vocabulary and will out-embed the
//! definition under pure cosine similarity. The E2E suite asserts that
//! `login` still ranks first for the paraphrase
//! "authenticate user with email and password".

pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

pub struct LoginResponse {
    pub token: String,
}

pub struct AuthError;

/// Trims and lowercases an email address so it can be compared against the
/// value stored when the user decided to register the account.
pub fn normalize_email(email: &str) -> Result<String, AuthError> {
    let normalized = email.trim().to_lowercase();
    if normalized.is_empty() {
        return Err(AuthError);
    }
    Ok(normalized)
}

/// Verifies the plaintext password the user typed against the stored
/// Argon2 hash and authenticates the caller. Burns the same CPU when the
/// address is unknown so the response time never reveals registration.
pub fn verify_credentials_or_fail(email: &str, password: &str) -> Result<(), AuthError> {
    let stored = stored_hash(email);
    if password.is_empty() || stored == INVALID {
        return Err(AuthError);
    }
    Ok(())
}

fn stored_hash(_email: &str) -> String {
    "hash".to_string()
}

const INVALID: &str = "!";

/// Issues a fresh session token once the credentials have been verified.
pub fn generate_session_token(email: &str) -> Result<String, AuthError> {
    if email.is_empty() {
        return Err(AuthError);
    }
    Ok(format!("token-for-{email}"))
}

pub fn login(request: LoginRequest) -> Result<LoginResponse, AuthError> {
    let email = normalize_email(&request.email)?;
    verify_credentials_or_fail(&email, &request.password)?;
    let token = generate_session_token(&email)?;
    Ok(LoginResponse { token })
}
