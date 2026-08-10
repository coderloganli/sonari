#[path = "../adapters/mod.rs"]
pub mod adapters;
#[path = "../domain/mod.rs"]
pub mod domain;
#[path = "../ports/mod.rs"]
pub mod ports;

pub use auth_console_contract::{AdminConsoleAdminItem, AdminConsoleQueryPort};
pub use domain::{AccountStatus, Admin, AdminRole, SmsCode, UserIdentity};
