//! Each form pinned to the text the router writes or reads. The texts are the serde forms of
//! docket-core's types (`Invocation`, `Outcome`, `AppRefusal`, `Preview`, `Hit`, `Label`), read
//! from its source and its pinned-JSON tests; a form that changes there fails here, where it is
//! cheaper than at a person's desk.

use super::*;
use serde_json::{Value as Json, json};

fn id(kind: &str, key: &str) -> EntityId {
    EntityId {
        app: "org.quire.Mail".to_owned(),
        kind: kind.to_owned(),
        key: key.to_owned(),
    }
}

fn json_of<T: serde::Serialize>(value: &T) -> Json {
    serde_json::to_value(value).expect("json")
}

/// The router's label for what the person typed.
fn user() -> Json {
    json!({ "integrity": "trusted", "confidentiality": { "kind": "public" },
            "classes": [], "sources": [{ "kind": "user" }] })
}

fn label_of(text: Json) -> Label {
    serde_json::from_value(text).expect("a label")
}

/// A label mailo is given goes back out as it came, sources mailo has no name for included.
#[test]
fn a_label_from_the_router_is_read_and_written_unchanged() {
    let text = json!({
        "integrity": "untrusted",
        "confidentiality": { "kind": "private", "v": ["work"] },
        "classes": ["mail", "voice"],
        "sources": [{ "kind": "model", "v": "planner" }, { "kind": "mcp", "v": "editor" }, { "kind": "mail" }],
    });
    assert_eq!(json_of(&label_of(text.clone())), text);
    assert_eq!(
        label_of(
            json!({ "integrity": "trusted", "confidentiality": { "kind": "secret" },
                                "classes": [], "sources": [] })
        )
        .integrity(),
        Integrity::Trusted
    );
    assert!(
        serde_json::from_value::<Label>(json!({})).is_err(),
        "a label says what it is"
    );
}

/// The router's join: integrity to the lower, confidentiality to the higher with the Spaces
/// unioned and the desktop dropped beside a real one, sources and classes unioned.
#[test]
fn a_join_takes_the_worse_of_each_half() {
    let chosen = label_of(user());
    let book = Label::contacts("work");
    let joined = chosen.join(&book);
    assert_eq!(
        json_of(&joined),
        json!({ "integrity": "trusted", "confidentiality": { "kind": "private", "v": ["work"] },
                "classes": ["contacts"], "sources": [{ "kind": "user" }, { "kind": "contacts" }] })
    );
    let lifted = Label::mail("desktop").join(&book);
    assert_eq!(lifted.integrity(), Integrity::Untrusted);
    assert_eq!(
        json_of(&lifted)["confidentiality"],
        json!({ "kind": "private", "v": ["work"] })
    );
    let secret = label_of(
        json!({ "integrity": "trusted", "confidentiality": { "kind": "secret" },
                                  "classes": [], "sources": [] }),
    );
    assert_eq!(
        json_of(&book.join(&secret))["confidentiality"],
        json!({ "kind": "secret" })
    );
    assert_eq!(chosen.join(&chosen), chosen);
}

/// Written by docket-core's own `Invocation` (a `quire-do` call: the actor and origin are the
/// terminal's), serialized with its `serde_json`.
const FROM_THE_ROUTER: &str = r#"{"call":41,"action":"mail.thread.label","target":{"kind":"entities","v":[{"app":"org.quire.Mail","kind":"mail.thread","key":"t-1"}]},"args":{"label":{"value":{"kind":"text","v":"Work"},"label":{"integrity":"trusted","confidentiality":{"kind":"public"},"classes":[],"sources":[{"kind":"user"}]}},"until":{"value":{"kind":"date_time","v":1700000000},"label":{"integrity":"trusted","confidentiality":{"kind":"public"},"classes":[],"sources":[{"kind":"user"}]}}},"actor":{"kind":"cli"},"origin":"cli","space":"work"}"#;

/// A launcher's call carries the activation token it minted, at the invocation's top level.
#[test]
fn a_launchers_activation_token_is_read() {
    let mut text: Json = serde_json::from_str(FROM_THE_ROUTER).expect("json");
    text["activation"] = json!("tok-7");
    let invocation: Invocation = serde_json::from_value(text).expect("an invocation");
    assert_eq!(invocation.activation.as_deref(), Some("tok-7"));
}

#[test]
fn an_invocation_as_the_router_writes_it_is_read() {
    let invocation: Invocation = serde_json::from_str(FROM_THE_ROUTER).expect("an invocation");
    assert_eq!(invocation.call, 41);
    assert_eq!(invocation.action, "mail.thread.label");
    assert_eq!(
        invocation.target,
        Target::Entities(vec![id("mail.thread", "t-1")])
    );
    assert_eq!(invocation.text("label"), Some("Work"));
    assert_eq!(invocation.instant("until"), Some(1_700_000_000));
    assert_eq!(invocation.text("until"), None, "an instant is not text");
    assert_eq!(invocation.instant("label"), None, "text is not an instant");
    assert_eq!(
        invocation.activation, None,
        "only a launcher's call carries a token"
    );
    assert_eq!(invocation.space, "work");
}

#[test]
fn a_form_mailo_has_no_use_for_is_read_and_is_neither_text_nor_an_instant() {
    let text = json!({
        "call": 1, "action": "mail.thread.snooze", "target": { "kind": "nothing" },
        "args": {
            "size": { "value": { "kind": "decimal", "v": { "units": 5, "scale": 1 } }, "label": user() },
            "kinds": { "value": { "kind": "list", "v": [{ "kind": "text", "v": "a" }] }, "label": user() },
        },
        "actor": { "kind": "companion", "v": { "session": "s-1", "role": { "kind": "planner" } } },
        "origin": "companion", "space": "work",
    })
    .to_string();
    let invocation: Invocation = serde_json::from_str(&text).expect("an invocation");
    for name in ["size", "kinds"] {
        assert_eq!(invocation.text(name), None);
        assert_eq!(invocation.instant(name), None);
    }
}

#[test]
fn every_target_the_router_has_is_read() {
    for (text, target) in [
        (json!({ "kind": "nothing" }), Target::Nothing),
        (
            json!({ "kind": "text", "v": "field-1" }),
            Target::Text("field-1".to_owned()),
        ),
        (
            json!({ "kind": "files", "v": ["/a"] }),
            Target::Files(vec!["/a".to_owned()]),
        ),
    ] {
        assert_eq!(
            serde_json::from_value::<Target>(text).expect("a target"),
            target
        );
    }
}

#[test]
fn an_outcome_is_written_as_the_router_reads_it() {
    let outcome = Outcome {
        value: Some(Labelled {
            value: Output::Entities(vec![id("mail.thread", "t-1")]),
            label: Label::own("org.quire.Mail"),
        }),
        said: Some("Archived".to_owned()),
        show: Preview::None,
        undo: Undoable::Yes("stack-3".to_owned()),
        follow: Follow::Nothing,
    };
    assert_eq!(
        json_of(&outcome),
        json!({
            "value": {
                "value": { "kind": "entities", "v": [{ "app": "org.quire.Mail", "kind": "mail.thread", "key": "t-1" }] },
                "label": { "integrity": "trusted", "confidentiality": { "kind": "public" },
                           "classes": [], "sources": [{ "kind": "app", "v": "org.quire.Mail" }] },
            },
            "said": "Archived",
            "show": { "kind": "none" },
            "undo": { "kind": "yes", "v": "stack-3" },
            "follow": { "kind": "nothing" },
        })
    );
    assert_eq!(json_of(&Undoable::No), json!({ "kind": "no" }));
}

#[test]
fn mail_is_labelled_as_somebody_elses_words_private_to_the_space() {
    assert_eq!(
        json_of(&Label::mail("work")),
        json!({ "integrity": "untrusted",
                "confidentiality": { "kind": "private", "v": ["work"] },
                "classes": ["mail"],
                "sources": [{ "kind": "mail" }] })
    );
    // Debug never shows the words.
    let said = Labelled {
        value: "a secret".to_owned(),
        label: Label::mail("work"),
    };
    assert!(!format!("{said:?}").contains("secret"));
}

#[test]
fn refusals_are_written_as_the_router_reads_them() {
    let thread = id("mail.thread", "t-1");
    for (refusal, expected) in [
        (AppRefusal::Busy, json!({ "kind": "busy" })),
        (AppRefusal::Unsupported, json!({ "kind": "unsupported" })),
        (
            AppRefusal::Failed("no such label".to_owned()),
            json!({ "kind": "failed", "v": "no such label" }),
        ),
        (
            AppRefusal::NotFound(thread.clone()),
            json!({ "kind": "not_found", "v": { "app": "org.quire.Mail", "kind": "mail.thread", "key": "t-1" } }),
        ),
        (
            AppRefusal::NeedsParam {
                param: "from".to_owned(),
                options: vec![],
            },
            json!({ "kind": "needs_param", "v": { "param": "from", "options": [] } }),
        ),
    ] {
        assert_eq!(json_of(&refusal), expected);
    }
    assert_eq!(json_of(&UndoFault::Gone), json!("gone"));
    assert_eq!(json_of(&UndoFault::Conflict), json!("conflict"));
}

#[test]
fn answers_are_ok_and_err_with_nothing_as_null() {
    let done: Result<(), UndoFault> = Ok(());
    let gone: Result<(), UndoFault> = Err(UndoFault::Gone);
    assert_eq!(answer_of(&done), r#"{"Ok":null}"#);
    assert_eq!(answer_of(&gone), r#"{"Err":"gone"}"#);
    let refused: Result<Preview, AppRefusal> = Err(AppRefusal::Busy);
    assert_eq!(answer_of(&refused), r#"{"Err":{"kind":"busy"}}"#);
}

#[test]
fn previews_hits_and_the_context_are_written_as_the_router_reads_them() {
    let own = |value: &str| Labelled {
        value: value.to_owned(),
        label: Label::own("org.quire.Mail"),
    };
    let message = Preview::Message {
        to: vec![own("ada@b.c")],
        subject: own("Hi"),
        body: own("Text"),
    };
    let written = json_of(&message);
    assert_eq!(written["kind"], "message");
    assert_eq!(written["v"]["to"][0]["value"], "ada@b.c");
    assert_eq!(written["v"]["subject"]["value"], "Hi");

    let thread = Preview::Thread {
        subject: own("Lunch"),
        messages: vec![Snip {
            from: own("Ada"),
            snippet: own("noon?"),
            at: 1_700_000_000,
        }],
    };
    assert_eq!(json_of(&thread)["v"]["messages"][0]["at"], 1_700_000_000);

    let hit = Hit {
        entity: EntityRef {
            id: id("mail.thread", "t-1"),
            title: own("Lunch"),
            subtitle: own("Ada"),
        },
        why: None,
    };
    let written = json_of(&hit);
    assert_eq!(written["entity"]["id"]["kind"], "mail.thread");
    assert_eq!(written["why"], Json::Null);

    let context = Context {
        app: "org.quire.Mail".to_owned(),
        window: own(""),
        here: Here::Nowhere,
        selection: Selection::Nothing,
        visible: Visible {
            kind: None,
            items: vec![],
            total: 0,
        },
        text_target: TextTarget::None,
        privacy: Privacy::Private,
    };
    let written = json_of(&context);
    assert_eq!(written["here"], json!({ "kind": "nowhere" }));
    assert_eq!(written["selection"], json!({ "kind": "nothing" }));
    assert_eq!(
        written["visible"],
        json!({ "kind": null, "items": [], "total": 0 })
    );
    assert_eq!(written["text_target"], json!({ "kind": "none" }));
    assert_eq!(written["privacy"], "private");
}

#[test]
fn a_suggest_ask_is_read() {
    let ask: SuggestAsk = serde_json::from_value(json!({
        "action": { "app": "org.quire.Mail", "name": "mail.thread.label" },
        "param": "label",
        "typed": "wo",
    }))
    .expect("an ask");
    assert_eq!(
        (
            ask.action.name.as_str(),
            ask.param.as_str(),
            ask.typed.as_str()
        ),
        ("mail.thread.label", "label", "wo")
    );
}
