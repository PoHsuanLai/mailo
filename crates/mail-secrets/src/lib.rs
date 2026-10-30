//! The credentials: account secrets, signing keys and the tokens a sign-in leaves behind.
//!
//! Where a secret is kept (the platform's keyring, or the Secret Service through oo7), which
//! OAuth client this build signs in as, how a sign-in is run and an access token kept fresh.
//! It reaches the network only for the issuer's own endpoints (through `mail-net`) and names
//! no store: `mail-store` is not among its dependencies, which `scripts/check-boundary.sh`
//! holds it to.

pub mod account_secrets;
mod atomic;
pub mod authorize;
pub mod clients;
mod error;
pub mod no_keyring;
pub mod places;
pub mod signing_store;
#[cfg(feature = "testing")]
pub mod testing;
pub mod tokens;

pub use account_secrets::{
    AccountSecrets, PlatformSecrets, SecretTrouble, block_on, platform_secrets,
};
pub use error::CredentialError;
pub use no_keyring::NoKeyring;
pub use porter_oauth::ClientRegistry;
pub use signing_store::{KeyringSigningStore, SigningStore, off_runtime};
pub use tokens::{AfterRefusal, Held, OAuthTokens, Token, TokenSource};
