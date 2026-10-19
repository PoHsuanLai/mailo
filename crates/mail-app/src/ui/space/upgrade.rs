//! A `spaces.json` from before Spaces named their accounts, brought up to date as it is read.
//!
//! Such a Space kept a `scope`: some accounts, or every account (`{"kind":"all"}`), which also
//! took in each account added later. It now names its accounts: the ones its scope listed, or,
//! for every account, the accounts the store has as it is read, which is what it showed. A
//! Space of another Profile, whose store is not open here, cannot be told its accounts, and
//! names none.

use ds_settings::Fixup;
use mail_core::profile::ProfileId;
use porter_core::AccountId;
use serde_json::{Map, Value};

/// Give each Space in `file` that has no `accounts` the ones its old `scope` stood for, `known`
/// standing for every account in the default Profile. Whether any Space changed.
pub(super) fn name_accounts(file: &mut Value, known: &[AccountId]) -> Fixup {
    let Some(list) = file.get_mut("spaces").and_then(Value::as_array_mut) else {
        return Fixup::Kept;
    };
    let mut fixup = Fixup::Kept;
    for space in list.iter_mut().filter_map(Value::as_object_mut) {
        if space.contains_key("accounts") {
            continue;
        }
        let accounts = accounts_of(space, known);
        space.remove("scope");
        space.insert("accounts".to_owned(), Value::Array(accounts));
        fixup = Fixup::Changed;
    }
    fixup
}

/// The accounts a Space's old `scope` stood for. A scope this build cannot read was read as
/// every account, as is a Space that kept none.
fn accounts_of(space: &Map<String, Value>, known: &[AccountId]) -> Vec<Value> {
    let listed = space
        .get("scope")
        .filter(|scope| scope.get("kind").and_then(Value::as_str) == Some("accounts"))
        .and_then(|scope| scope.get("v"))
        .and_then(Value::as_array);
    if let Some(listed) = listed {
        return listed.clone();
    }
    let profile = space
        .get("profile")
        .and_then(|profile| serde_json::from_value::<ProfileId>(profile.clone()).ok())
        .unwrap_or_default();
    if profile != ProfileId::default() {
        return Vec::new();
    }
    known
        .iter()
        .filter_map(|id| serde_json::to_value(id).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::name_accounts;
    use ds_settings::Fixup;
    use porter_core::AccountId;
    use serde_json::{Value, json};
    use uuid::Uuid;

    fn account(n: u128) -> AccountId {
        mail_domain::id::account_id_from_uuid(Uuid::from_u128(n))
    }

    fn id(n: u128) -> Value {
        serde_json::to_value(account(n)).unwrap_or_else(|e| panic!("{e}"))
    }

    #[test]
    fn each_old_scope_names_the_accounts_it_showed() {
        let known = [account(1), account(2)];
        // (the Space as stored, its accounts after)
        let cases: Vec<(Value, Value)> = vec![
            (json!({"scope": {"kind": "all"}}), json!([id(1), id(2)])),
            (
                json!({"scope": {"kind": "accounts", "v": [id(2)]}}),
                json!([id(2)]),
            ),
            (json!({"scope": {"kind": "accounts", "v": []}}), json!([])),
            // Unreadable or missing was every account.
            (json!({"scope": {"kind": "team"}}), json!([id(1), id(2)])),
            (json!({"name": "A"}), json!([id(1), id(2)])),
            // Another Profile's store is not open here.
            (
                json!({"profile": "0123abcd", "scope": {"kind": "all"}}),
                json!([]),
            ),
        ];
        for (space, want) in cases {
            let mut file = json!({ "spaces": [space.clone()] });
            assert_eq!(name_accounts(&mut file, &known), Fixup::Changed, "{space}");
            let after = &file["spaces"][0];
            assert_eq!(after["accounts"], want, "{space}");
            assert!(after.get("scope").is_none(), "{space}: the scope stayed");
        }
    }

    #[test]
    fn a_space_that_names_its_accounts_is_left_as_it_is() {
        let mut file = json!({ "spaces": [{"accounts": [id(3)], "scope": {"kind": "all"}}] });
        let before = file.clone();
        assert_eq!(name_accounts(&mut file, &[account(1)]), Fixup::Kept);
        assert_eq!(file, before);
    }
}
