use transit_core::{error, oneof, record, route};

use crate::{
    auth::{Authentication, AuthenticationStrict},
    error::Denied,
};

#[record]
pub struct User {
    pub uid: String,
    pub username: String,
    pub email: String,
}

#[oneof]
pub enum UserQuery {
    Uid(String),
    Username(String),
    Email(String),
}

#[record]
pub struct UserSearchRequest {
    pub auth: Authentication,
    pub query: UserQuery,
}

#[record]
pub struct UserUpdateRequest {
    pub auth: Authentication,
    pub query: UserQuery,
    pub username: Option<String>,
    pub email: Option<String>,
}

#[record]
pub struct UserUpdateResponse {
    pub user: User,
    pub jwt: Option<String>,
}

#[record]
pub struct UpdatePasswordRequest {
    pub auth: AuthenticationStrict,
    pub query: UserQuery,
    pub password: String,
}

#[oneof]
pub enum UserDeleteRequest {
    DeleteSelf { uid: String, password: String },
    DeleteOther { auth: Authentication, uid: String },
}

error! {
    NoUser("User not found");

    InvalidEmail("Invalid email");
    InvalidUsername("Invalid username");
    EmailInUse("Email in use");
    UsernameInUse("Username in use");

    UserListError = Denied;
    UserSearchError = NoUser | Denied;
    UserUpdateError = NoUser | InvalidUsername | InvalidEmail | UsernameInUse | EmailInUse | Denied;
    UpdatePasswordError = NoUser | Denied;
    UserDeleteError = NoUser | Denied;
}

route! {
    UserList(Authentication) -> Result<Vec<User>, UserListError>;
    UserSearch(UserSearchRequest) -> Result<User, UserSearchError>;
    UserUpdate(UserUpdateRequest) -> Result<UserUpdateResponse, UserUpdateError>;
    UpdatePassword(UpdatePasswordRequest) -> Result<UserUpdateResponse, UpdatePasswordError>;
    UserDelete(UserDeleteRequest) -> Result<(), UserDeleteError>;
}
