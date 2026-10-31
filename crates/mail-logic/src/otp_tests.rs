//! What [`code`] finds, and what it leaves alone.

use super::*;

/// Codes as services send them, in the languages the keywords cover.
#[test]
fn a_code_near_its_keyword_is_found() {
    // (what it is, subject, body, the code)
    let cases: [(&str, &str, &str, &str); 30] = [
        (
            "after its keyword",
            "Your verification code",
            "Your verification code is 482913.",
            "482913",
        ),
        (
            "before its keyword, in the subject",
            "482913 is your Instagram code",
            "",
            "482913",
        ),
        (
            "Google's letter and hyphen",
            "G-482913 is your Google verification code",
            "",
            "G-482913",
        ),
        (
            "four digits",
            "Sign in to Example",
            "Use this one-time code to sign in: 9157",
            "9157",
        ),
        (
            "eight digits",
            "",
            "Your OTP is 12345678. Do not share it.",
            "12345678",
        ),
        (
            "two groups of letters",
            "Security code",
            "Your security code: ABC-DEF",
            "ABC-DEF",
        ),
        (
            "a group of letters and digits",
            "",
            "Enter the code K7Q-29XD to continue.",
            "K7Q-29XD",
        ),
        (
            "the subject's keyword, the body's code",
            "Login attempt",
            "Enter 739201 to finish.",
            "739201",
        ),
        (
            "on a line of its own under a greeting",
            "Verify your email",
            "Hi Ada,\n\n482913\n\nThis code expires in 10 minutes.",
            "482913",
        ),
        ("a passcode", "", "Passcode: 30418822", "30418822"),
        (
            "past a year in the footer",
            "© 2026 Example",
            "Your verification code is 482913",
            "482913",
        ),
        ("a plural keyword", "", "Backup codes: 55102938", "55102938"),
        (
            "a sign-in code",
            "",
            "Your sign-in code\n\n730195",
            "730195",
        ),
        (
            "Simplified Chinese",
            "验证码",
            "您的验证码为：583920，5分钟内有效。",
            "583920",
        ),
        (
            "Traditional Chinese",
            "",
            "您的驗證碼是 583920，請勿告訴他人。",
            "583920",
        ),
        ("認證碼", "", "認證碼：8812", "8812"),
        ("動態密碼", "", "動態密碼 66120394", "66120394"),
        ("动态密码", "", "您的动态密码是 77129043", "77129043"),
        (
            "Japanese",
            "",
            "認証コード：123987 を入力してください。",
            "123987",
        ),
        (
            "Japanese in full-width digits",
            "",
            "確認コードは ４４５５６６ です。",
            "445566",
        ),
        (
            "German",
            "Ihr Bestätigungscode",
            "Ihr Bestätigungscode lautet 284615.",
            "284615",
        ),
        ("German's plain Code", "", "Ihr Code: 5521", "5521"),
        (
            "Spanish",
            "",
            "Tu código de verificación es 908172",
            "908172",
        ),
        (
            "French",
            "",
            "Votre code de vérification : 771204",
            "771204",
        ),
        (
            "the code, not the order before it",
            "Your order",
            "Order 12345678 is ready. Your code: 482913",
            "482913",
        ),
        (
            "the code, not the account it is for",
            "",
            "Your code for account 1234567 is 482913.",
            "482913",
        ),
        (
            "with a non-breaking space",
            "",
            "Your code:\u{a0}482913",
            "482913",
        ),
        (
            "in a sentence with a time in it",
            "",
            "At 10:30 you asked to verify. Code: 6620",
            "6620",
        ),
        (
            "after a link to somewhere else",
            "",
            "Verify at https://example.test/v and enter 482913",
            "482913",
        ),
        (
            "upper-case keyword",
            "YOUR ONE-TIME CODE",
            "YOUR ONE-TIME CODE IS 909090",
            "909090",
        ),
    ];
    for (name, subject, body, want) in cases {
        assert_eq!(code(subject, body).as_deref(), Some(want), "{name}");
    }
}

/// Numbers that are not codes, beside a keyword or not.
#[test]
fn a_number_that_is_something_else_is_not_a_code() {
    // (what it is, subject, body)
    let cases: [(&str, &str, &str); 32] = [
        (
            "an order number, no keyword",
            "Your order has shipped",
            "Order #48291377 is on its way.",
        ),
        (
            "an order number and a price",
            "Your receipt",
            "Order number 482913. Total $1234.00",
        ),
        (
            "telephone numbers",
            "Verify your account",
            "Call us at +1 415 555 0123 or 415-555-0123.",
        ),
        (
            "a telephone number in groups",
            "Security code",
            "Call 555 1234 to reset.",
        ),
        (
            "a date and a time",
            "Sign in",
            "You signed in on 2026-10-24 at 10:30.",
        ),
        (
            "a dotted date",
            "Login alert",
            "A new login from Chrome on 24.10.2026.",
        ),
        (
            "a slashed date",
            "Verify",
            "Requested on 10/24/2026 from Taipei.",
        ),
        ("a year", "Your code", "Valid until 2027."),
        (
            "a price and a year",
            "Your code",
            "Total: €1299 paid in 2025.",
        ),
        ("a price with its cents", "Login", "Price 1,299.00 USD"),
        ("a price in words", "Login", "You were charged 4500 USD."),
        ("a percentage", "Verify", "Save 1500% now"),
        (
            "a promo code",
            "Your promo code",
            "Use promo code 4521 for 20% off.",
        ),
        (
            "a zip code",
            "Zip code update",
            "Your zip code 94103 was saved.",
        ),
        ("an error code", "", "Error code 4012 occurred."),
        ("a code postal", "", "Code postal 75011"),
        ("un código postal", "", "Código postal 28013"),
        ("a code review", "Code review", "PR 4521 needs a look."),
        (
            "an account number",
            "Verification complete",
            "Thanks! Your account 12345678 is verified.",
        ),
        ("a login ID", "Login", "Your login ID 55667788"),
        (
            "a number in a link",
            "Sign-in",
            "Visit https://example.test/verify/48291377 to continue",
        ),
        ("a number in a query", "", "verify?id=482913"),
        ("a keyword alone", "Your verification code", ""),
        ("too short", "Your code", "Code 123"),
        ("too long", "Your code", "Code 123456789"),
        (
            "a number with no keyword",
            "Meeting notes",
            "We spent 123456 hours on it.",
        ),
        (
            "a number labelled in Chinese",
            "验证码",
            "订单 482913 已发货",
        ),
        ("a telephone in Chinese", "验证码", "客服电话 40088123"),
        ("barcode is not code", "", "Barcode 48291377"),
        (
            "a word, not two groups",
            "VERIFY YOUR E-MAIL",
            "SIGN-IN NOW",
        ),
        (
            "a keyword too far away",
            "",
            "Verify your email address so we know it is you. Thank you for joining us here, \
             it means a great deal to everyone on the team. 482913",
        ),
        ("letters run into the digits", "Your code", "Code AB482913"),
    ];
    for (name, subject, body) in cases {
        assert_eq!(code(subject, body), None, "{name}");
    }
}

/// A long message is read only as far as a code would be.
#[test]
fn a_code_past_the_top_of_a_long_message_is_not_looked_for() {
    let long = format!("{}Your code is 482913", "lorem ipsum ".repeat(500));
    assert_eq!(code("Newsletter", &long), None);
    assert_eq!(
        code("Newsletter", &format!("Your code is 482913 {long}")).as_deref(),
        Some("482913")
    );
}
