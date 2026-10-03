use transit_core::{error, oneof, record, route};

use crate::user::{NoUser, UserQuery};

#[oneof]
pub enum Authentication {
    Token(String),
}

#[oneof]
pub enum AuthenticationStrict {
    Password(String),
}

#[record]
pub struct AuthenticateRequest {
    pub query: UserQuery,
    pub password: String,
}

#[record]
pub struct AuthenticateResponse {
    pub jwt: String,
}

error! {
    BadPassword("Incorrect password");
    AuthenticateError = NoUser | BadPassword;
}

route! {
    Authenticate(AuthenticateRequest) -> Result<AuthenticateResponse, AuthenticateError>;
}
