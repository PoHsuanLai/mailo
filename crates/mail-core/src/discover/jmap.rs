//! `account add <address> --jmap` with no URL: the domain's `/.well-known/jmap` (RFC 8620 §2.2),
//! found without a credential. Confirmed before one is sent by the command line
//! (`mail_app::cli::discover`).

/// Follow `url` over the network, without a credential.
pub fn find(url: &str) -> Result<String, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the async runtime: {e}"))?;
    runtime.block_on(async {
        let http = mail_runtime::discover::client_builder()
            .build()
            .map_err(|e| format!("cannot build an HTTP client: {e}"))?;
        mail_runtime::jmap::find_session(&http, url)
            .await
            .map_err(|e| e.to_string())
    })
}
