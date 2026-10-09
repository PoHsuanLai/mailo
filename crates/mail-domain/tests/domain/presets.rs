//! Which submission servers file the sender's copy themselves.

use mail_domain::presets::files_sent_itself;

#[test]
fn only_servers_known_to_file_the_copy_are_taken_to() {
    const CASES: &[(&str, bool)] = &[
        ("smtp.gmail.com", true),
        ("SMTP.GMAIL.COM", true),
        ("smtp.googlemail.com", true),
        ("smtp.office365.com", true),
        ("smtp-mail.outlook.com", true),
        // Anything else files nothing: a plain Dovecot or Postfix, a host on a lookalike name.
        ("mail.example.test", false),
        ("127.0.0.1", false),
        ("smtp.evil-gmail.com", false),
        ("gmail.com.example.test", false),
    ];
    for (host, files) in CASES {
        assert_eq!(files_sent_itself(host), *files, "{host}");
    }
}
