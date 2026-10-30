//! `mailo offline [<address> [on|off]]`: set one account, or say where each stands.

use super::Store;
use mail_core::error::CoreError;
use mail_core::offline::{Keep, Standing, said, standing};
use porter_core::AccountId;
use std::path::Path;

/// Run `mailo offline …`. `dir` is `None` when there is no home directory to keep the setting
/// in; then only a request to change it fails.
pub fn command(
    dir: Option<&Path>,
    store: &dyn Store,
    accounts: &[(AccountId, String)],
    address: Option<&str>,
    set: Option<Keep>,
) -> Result<String, CoreError> {
    Ok(render(&standing(dir, store, accounts, address, set)?))
}

/// One line for each account, or how to add one when there are none.
pub fn render(accounts: &[Standing]) -> String {
    let mut out = String::new();
    for account in accounts {
        let keeps = match account.keep {
            Keep::Everything => "all mail kept offline",
            Keep::Bodies => "large attachments left on the server until opened",
        };
        out.push_str(&format!(
            "{}: {keeps}; {}\n",
            account.address,
            said(&account.counted)
        ));
    }
    if out.is_empty() {
        out.push_str("no accounts. Add one with: mailo account add <address>\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_account_gets_a_line_and_none_says_how_to_add_one() {
        let standing = Standing {
            address: "me@example.test".to_owned(),
            keep: Keep::Everything,
            counted: Default::default(),
        };
        assert_eq!(
            render(std::slice::from_ref(&standing)),
            "me@example.test: all mail kept offline; 0 of 0 messages offline\n"
        );
        let bodies = Standing {
            keep: Keep::Bodies,
            ..standing
        };
        assert_eq!(
            render(&[bodies]),
            "me@example.test: large attachments left on the server until opened; \
             0 of 0 messages offline\n"
        );
        assert_eq!(
            render(&[]),
            "no accounts. Add one with: mailo account add <address>\n"
        );
    }
}
