//! Test fixture: test-file entities must not outrank the production
//! definition for the same behaviour.

use search_rank::{LoginRequest, login};

#[test]
fn social_login_with_email_and_password() {
    let request = LoginRequest {
        email: "user@example.com".to_string(),
        password: "secret".to_string(),
    };
    assert!(login(request).is_ok());
}
