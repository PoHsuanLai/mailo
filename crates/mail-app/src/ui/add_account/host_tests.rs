//! porter's account service hosted in process, the window's end of it driven by hand: a whole
//! password add and a whole browser add over mailo's providers, with the fakes of
//! `provider_tests`; the sheet closing when the person leaves; nothing added by a dismissal.

use std::sync::Arc;
use std::time::Duration;

use mail_core::discover::{Failed, Gap};
use porter_core::sheet::ServiceChoice;
use porter_core::sheet::{
    FieldAnswer, FieldKind, FieldValue, ProblemKind, SheetInput, SheetView, SignInFault,
};
use porter_core::{CapabilityKind, ProviderId, SecretText, Toggle};

use super::host::{self, Ended, Shown, WindowEnd};
use super::provider::{Added, offered};
use super::provider_tests::{AUTHORIZE, Log, PASSWORD, Script, seams};

/// Wait for the first view `is` holds for, which the service shows after what was sent.
async fn view(end: &mut WindowEnd, is: impl Fn(&SheetView) -> bool) -> SheetView {
    let wait = async {
        loop {
            if let Shown::View(view) = end.views.borrow_and_update().clone()
                && is(&view)
            {
                return view;
            }
            end.views.changed().await.expect("the service ended first");
        }
    };
    tokio::time::timeout(Duration::from_secs(20), wait)
        .await
        .expect("the view never came")
}

fn pick(id: &str) -> SheetInput {
    SheetInput::Pick(ProviderId::parse(id).unwrap())
}

fn submit(answers: Vec<(FieldKind, FieldValue)>) -> SheetInput {
    SheetInput::Submit(
        answers
            .into_iter()
            .map(|(kind, value)| FieldAnswer { kind, value })
            .collect(),
    )
}

fn confirm() -> SheetInput {
    SheetInput::Confirm(vec![ServiceChoice {
        kind: CapabilityKind::Mail,
        toggle: Toggle::On,
    }])
}

/// The service running over the fakes: its window end, the join of `run`, and what it counted.
fn serve(script: &Script) -> (WindowEnd, tokio::task::JoinHandle<Ended>, Arc<Log>, Added) {
    let log = Arc::new(Log::default());
    let added = Added::default();
    let (end, sheets) = host::ends();
    let providers = offered(&seams(script, &log), &added, None);
    let task = tokio::spawn(host::run(providers, sheets));
    (end, task, log, added)
}

#[tokio::test]
async fn a_whole_password_add_over_the_service() {
    let (mut end, task, log, added) = serve(&Script::default());
    let list = view(&mut end, |v| matches!(v, SheetView::Providers(_))).await;
    let SheetView::Providers(rows) = list else {
        unreachable!()
    };
    assert_eq!(rows.len(), 7);

    end.inputs.send(pick("generic-imap")).unwrap();
    view(&mut end, |v| matches!(v, SheetView::SignIn(_))).await;
    end.inputs
        .send(submit(vec![
            (
                FieldKind::Address,
                FieldValue::Plain("ada@example.test".to_owned()),
            ),
            (
                FieldKind::Password,
                FieldValue::Secret(SecretText::new(PASSWORD)),
            ),
        ]))
        .unwrap();
    view(&mut end, |v| matches!(v, SheetView::Review(_))).await;
    assert!(
        log.adds.lock().unwrap().is_empty(),
        "nothing is added before Confirm"
    );
    end.inputs.send(confirm()).unwrap();

    assert_eq!(task.await.unwrap(), Ended::Added);
    assert_eq!(
        *log.adds.lock().unwrap(),
        [(
            "ada@example.test".to_owned(),
            Some(PASSWORD.to_owned()),
            false
        )]
    );
    assert_eq!(added.take().as_deref(), Some("ada@example.test"));
    assert!(matches!(end.views.borrow().clone(), Shown::Closed));
}

#[tokio::test]
async fn a_whole_browser_add_over_the_service_and_open_again_shows_the_page_again() {
    let script = Script::default();
    let (mut end, task, log, _added) = serve(&script);
    view(&mut end, |v| matches!(v, SheetView::Providers(_))).await;
    end.inputs.send(pick("google-mail")).unwrap();
    view(&mut end, |v| matches!(v, SheetView::SignIn(_))).await;
    end.inputs
        .send(submit(vec![(
            FieldKind::Address,
            FieldValue::Plain("ada@gmail.com".to_owned()),
        )]))
        .unwrap();
    let page = view(&mut end, |v| matches!(v, SheetView::BrowserWait { .. })).await;
    let SheetView::BrowserWait { url, .. } = &page else {
        unreachable!()
    };
    assert_eq!(url.as_str(), AUTHORIZE);

    // "Open Again": the same page, shown again.
    end.views.borrow_and_update();
    end.inputs.send(SheetInput::OpenAgain).unwrap();
    tokio::time::timeout(Duration::from_secs(20), end.views.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(end.views.borrow_and_update().clone(), Shown::View(page));

    script.release.notify_one();
    view(&mut end, |v| matches!(v, SheetView::Review(_))).await;
    end.inputs.send(confirm()).unwrap();
    assert_eq!(task.await.unwrap(), Ended::Added);
    assert_eq!(
        *log.adds.lock().unwrap(),
        [("ada@gmail.com".to_owned(), None, true)]
    );
}

#[tokio::test]
async fn a_failed_step_tries_again_from_the_form() {
    let script = Script {
        add: Err("cannot save the account".to_owned()),
        ..Script::default()
    };
    let (mut end, task, _log, added) = serve(&script);
    view(&mut end, |v| matches!(v, SheetView::Providers(_))).await;
    end.inputs.send(pick("generic-imap")).unwrap();
    view(&mut end, |v| matches!(v, SheetView::SignIn(_))).await;
    let form = || {
        submit(vec![
            (
                FieldKind::Address,
                FieldValue::Plain("ada@example.test".to_owned()),
            ),
            (
                FieldKind::Password,
                FieldValue::Secret(SecretText::new("pw")),
            ),
        ])
    };
    end.inputs.send(form()).unwrap();
    view(&mut end, |v| matches!(v, SheetView::Review(_))).await;
    end.inputs.send(confirm()).unwrap();
    let failed = view(&mut end, |v| matches!(v, SheetView::Failed { .. })).await;
    assert!(matches!(
        failed,
        SheetView::Failed {
            fault: SignInFault::StoreFailed,
            ..
        }
    ));

    end.inputs.send(SheetInput::Retry).unwrap();
    view(&mut end, |v| matches!(v, SheetView::SignIn(_))).await;
    end.inputs.send(SheetInput::Dismiss).unwrap();
    assert_eq!(task.await.unwrap(), Ended::NoAccount);
    assert_eq!(added.take(), None);
}

#[tokio::test]
async fn dismissing_adds_nothing_and_closing_the_window_ends_a_browser_wait() {
    let (mut end, task, log, _) = serve(&Script::default());
    view(&mut end, |v| matches!(v, SheetView::Providers(_))).await;
    end.inputs.send(SheetInput::Dismiss).unwrap();
    assert_eq!(task.await.unwrap(), Ended::NoAccount);
    assert!(log.adds.lock().unwrap().is_empty());

    let (mut end, task, log, _) = serve(&Script::default());
    view(&mut end, |v| matches!(v, SheetView::Providers(_))).await;
    end.inputs.send(pick("google-mail")).unwrap();
    view(&mut end, |v| matches!(v, SheetView::SignIn(_))).await;
    end.inputs
        .send(submit(vec![(
            FieldKind::Address,
            FieldValue::Plain("ada@gmail.com".to_owned()),
        )]))
        .unwrap();
    view(&mut end, |v| matches!(v, SheetView::BrowserWait { .. })).await;
    // The window goes away mid-wait.
    drop(end);
    assert_eq!(task.await.unwrap(), Ended::NoAccount);
    assert!(log.adds.lock().unwrap().is_empty());
}

fn typed(kind: FieldKind, text: &str) -> (FieldKind, FieldValue) {
    (kind, FieldValue::Plain(text.to_owned()))
}

/// A lookup that finds no server for the address.
fn nothing_found() -> Script {
    Script {
        lookup: Err(Failed::NoServers {
            address: "ada@example.test".to_owned(),
            gap: Gap::Nothing,
            tried: String::new(),
        }),
        ..Script::default()
    }
}

#[tokio::test]
async fn a_whole_typed_pop3_add_over_the_service() {
    let (mut end, task, log, added) = serve(&nothing_found());
    view(&mut end, |v| matches!(v, SheetView::Providers(_))).await;
    end.inputs.send(pick("generic-imap")).unwrap();
    view(&mut end, |v| matches!(v, SheetView::SignIn(_))).await;
    end.inputs
        .send(submit(vec![
            typed(FieldKind::Address, "ada@example.test"),
            (
                FieldKind::Password,
                FieldValue::Secret(SecretText::new(PASSWORD)),
            ),
        ]))
        .unwrap();

    // Nothing found: the form of typed servers, as IMAP first.
    let SheetView::SignIn(form) = view(&mut end, |v| {
        matches!(v, SheetView::SignIn(form) if form.fields.iter().any(|f| f.kind == FieldKind::Protocol))
    })
    .await
    else {
        unreachable!()
    };
    assert!(
        form.fields
            .iter()
            .any(|f| f.kind == FieldKind::OutgoingServer)
    );
    assert_eq!(log.lookups.lock().unwrap().len(), 1);

    // An unusable port is the machine's problem, before anything is sent anywhere.
    let answers = |port: &str| {
        submit(vec![
            typed(FieldKind::Protocol, "pop3"),
            typed(FieldKind::Server, "pop.example.test"),
            typed(FieldKind::Security, "tls"),
            typed(FieldKind::Port, port),
            typed(FieldKind::OutgoingServer, "smtp.example.test"),
            typed(FieldKind::OutgoingSecurity, "starttls"),
            typed(FieldKind::OutgoingPort, ""),
            typed(FieldKind::Username, ""),
        ])
    };
    end.inputs.send(answers("pop3")).unwrap();
    let SheetView::SignIn(form) = view(
        &mut end,
        |v| matches!(v, SheetView::SignIn(form) if form.problem.is_some()),
    )
    .await
    else {
        unreachable!()
    };
    let problem = form.problem.unwrap();
    assert_eq!(
        (problem.field, problem.problem),
        (FieldKind::Port, ProblemKind::Invalid)
    );

    end.inputs.send(answers("995")).unwrap();
    view(&mut end, |v| matches!(v, SheetView::Review(_))).await;
    assert!(
        log.adds.lock().unwrap().is_empty(),
        "nothing is added before Confirm"
    );
    end.inputs.send(confirm()).unwrap();
    assert_eq!(task.await.unwrap(), Ended::Added);
    assert_eq!(
        *log.adds.lock().unwrap(),
        [(
            "ada@example.test".to_owned(),
            Some(PASSWORD.to_owned()),
            false
        )]
    );
    assert_eq!(added.take().as_deref(), Some("ada@example.test"));
}
