//! Menu items and choices: a row in a menu, a segment of a segmented control, a picker's option.
//!
//! Case: Title Case verb phrases with no article ("Mark as Read", "Move to…"). "…" only when more input follows.

// The sidebar's menu, under the foot's chevron.
pub const SETTINGS: &str = "Settings…";
/// The sidebar's menu: archive the read conversations the Inbox has kept too long.
pub const TIDY_UP: &str = "Tidy Up…";
pub const HIDE_SIDEBAR: &str = "Hide Sidebar";
pub const SHOW_SIDEBAR: &str = "Show Sidebar";

// The snooze menu's times, then the row that asks for one typed.
pub const LATER_TODAY: &str = "Later Today";
pub const TOMORROW: &str = "Tomorrow";
pub const THIS_WEEKEND: &str = "This Weekend";
pub const NEXT_WEEK: &str = "Next Week";
pub const CHOOSE_TIME: &str = "Choose Time…";

// A scheduled message's menu: it goes back to Drafts, with undo, so nothing more is asked.
pub const CANCEL_SENDING: &str = "Cancel Sending";

// ⌘K's commands: the verbs the window runs, each listed with its key.
pub const NEW_MESSAGE: &str = "New Message";
/// Lists the templates in ⌘K itself.
pub const NEW_FROM_TEMPLATE: &str = "New from Template…";
pub const FORWARD_AS_ATTACHMENT: &str = "Forward as Attachment";
pub const PRINT_CONVERSATION: &str = "Print Conversation";
pub const EMPTY_TRASH: &str = "Empty Trash…";
pub const EMPTY_SPAM: &str = "Empty Spam…";
pub const IMPORT_MAIL: &str = "Import Mail…";
pub const EXPORT_MAIL: &str = "Export Mail…";
pub const NEW_VIEW: &str = "New View…";

// The Space's menu › Accounts.
pub const ACCOUNTS: &str = "Accounts";
/// Add Account when it opens the window itself.
pub const ADD_ACCOUNT: &str = "Add Account…";
/// Add Account when it is a submenu of the providers that sign in in the browser.
pub const ADD_ACCOUNT_SUBMENU: &str = "Add Account";

// The list toolbar's ⋯.
/// A submenu: the keys, then the two ways the chosen one runs ([`sort_key`], [`sort_dirs`]).
pub const SORT_BY: &str = "Sort By";
pub const GROUP_BY_DATE: &str = "Group by Date";
/// Checked while the list shows conversations; off, each message is a row of its own. Apple
/// Mail's words, from its View menu. Also a command in the search bar's panel.
pub const ORGANISE_BY_CONVERSATION: &str = "Organise by Conversation";
pub const SHOW_SNIPPETS: &str = "Show Snippets";
pub const HIDE_SNIPPETS: &str = "Hide Snippets";
pub const SHOW_LABELS: &str = "Show Labels";
pub const HIDE_LABELS: &str = "Hide Labels";
pub const SYNC_NOW: &str = "Sync Now";

// A folder's ⋯.
pub const NEW_FOLDER_INSIDE: &str = "New Folder Inside";
pub const RENAME: &str = "Rename";
pub const FOLLOW: &str = "Follow";
pub const STOP_FOLLOWING: &str = "Stop Following";
/// Under Stop Following: what it does beyond this window.
pub const STOP_FOLLOWING_HELP: &str = "Other mail apps may stop showing it.";
pub const DELETE: &str = "Delete";

// The row at the end of Folders that shows the folders nobody follows, or hides them again.
pub const SHOW_ALL_FOLDERS: &str = "Show All Folders";
pub const SHOW_FOLLOWED_ONLY: &str = "Show Followed Only";

// The composer's Send ▾ and ⋯.
pub const SEND_LATER: &str = "Send Later";
pub const REMIND_IF_NO_REPLY: &str = "Remind Me If No Reply";
pub const SECURITY: &str = "Security";
pub const SHOW_PLAIN_TEXT: &str = "Show Plain Text";
/// ⋯ › Priority ▸, and its three choices, most urgent first.
pub const PRIORITY: &str = "Priority";
pub const PRIORITY_HIGH: &str = "High";
pub const PRIORITY_NORMAL: &str = "Normal";
pub const PRIORITY_LOW: &str = "Low";
/// Asks the servers that carry the message to say when it is delivered, or why not.
pub const REQUEST_DELIVERY_RECEIPT: &str = "Request Delivery Receipt";
pub const SAVE_AS_TEMPLATE: &str = "Save as Template\u{2026}";
pub const KEEP_FOR_LATER: &str = "Keep for Later";
pub const DELETE_DRAFT: &str = "Delete Draft";

// A conversation's actions, in its row's menu, the reader's ⋯ and the selection bar: each named
// by the side the conversation is offered (`mail_client::conversation::Say`), never the side it is on.
pub const ARCHIVE: &str = "Archive";
/// Out of Archive or Trash, back to the inbox.
pub const MOVE_TO_INBOX: &str = "Move to Inbox";
/// Out of Spam, back to the inbox.
pub const NOT_SPAM: &str = "Not Spam";
pub const MARK_AS_SPAM: &str = "Mark as Spam";
pub const MOVE_TO_TRASH: &str = "Move to Trash";
pub const DELETE_FOREVER: &str = "Delete Forever…";
pub const MARK_AS_READ: &str = "Mark as Read";
pub const MARK_AS_UNREAD: &str = "Mark as Unread";
pub const STAR: &str = "Star";
pub const UNSTAR: &str = "Unstar";
/// The submenu of flag colours, beside Star or Unstar.
pub const FLAG: &str = "Flag";
/// The flag submenu's last row: unflags, as Unstar does.
pub const CLEAR_FLAG: &str = "Clear Flag";
pub const PIN: &str = "Pin";
pub const UNPIN: &str = "Unpin";
pub const MUTE: &str = "Mute";
pub const UNMUTE: &str = "Unmute";
pub const SNOOZE: &str = "Snooze…";
pub const UNSNOOZE: &str = "Unsnooze";
pub const REMIND: &str = "Remind Me If No Reply…";
pub const STOP_REMINDING: &str = "Stop Reminding";
pub const LABEL: &str = "Label…";
pub const REPLY: &str = "Reply";
pub const REPLY_ALL: &str = "Reply All";
pub const FORWARD: &str = "Forward";
/// The reader's ⋯ and ⌘K, when the message names its list's address (`List-Post`).
pub const REPLY_TO_LIST: &str = "Reply to List";
/// The reader's ⋯ and ⌘K: the message itself, unchanged, to someone else. Asks whom.
pub const REDIRECT: &str = "Redirect…";
/// The reader's ⋯ and ⌘K: a new message filled in from this one.
pub const EDIT_AS_NEW: &str = "Edit as New";
/// [`EDIT_AS_NEW`] for a conversation only in Sent, where the new message is the old one again.
pub const SEND_AGAIN: &str = "Send Again";
pub const MOVE_TO: &str = "Move to…";
/// Into another folder of the account as well, and left where it is.
pub const COPY_TO: &str = "Copy to…";
pub const DOWNLOAD: &str = "Download";
/// An attachment's right click: Save writes it where Save always does.
pub const SAVE: &str = "Save";
/// An attachment's right click: into a folder the person chooses.
pub const SAVE_TO: &str = "Save to…";
/// An attachment's right click, when its message has more than one.
pub const SAVE_ALL: &str = "Save All…";
pub const PRINT: &str = "Print…";
/// The reader's ⋯ and a message's own menu: the message's header lines as they came.
pub const SHOW_ALL_HEADERS: &str = "Show All Headers";
/// The reader's ⋯ and a message's own menu: the whole message as it came, headers and body.
pub const VIEW_SOURCE: &str = "View Source";
pub const VIEW: &str = "View";
/// The reader's ⋯, when the conversation's list offers a way out: it asks before it acts.
pub const UNSUBSCRIBE: &str = "Unsubscribe…";
/// The reader's ⋯: the sender's mail goes to Spam from now on, and this conversation with it.
pub const BLOCK_SENDER: &str = "Block Sender";
/// The same, naming whose mail it is: the newest sender who is not the user.
pub fn block_sender(email: &str) -> String {
    format!("Block {email}")
}

// The pick lists Move to… and Copy to… open: each one's name, which its field shares.
pub const MOVE_TO_LIST: &str = "Move to";
pub const COPY_TO_LIST: &str = "Copy to";

/// Copy to…'s folders, under the account's address: a copy goes to another folder of the same
/// account, never to another account.
pub fn copy_within(address: &str) -> String {
    if address.is_empty() {
        "This Account’s Folders".to_owned()
    } else {
        format!("Folders in {address}")
    }
}

/// A flag's or a label's colour, as Apple Mail names its seven.
pub fn hue(hue: mail_domain::Hue) -> &'static str {
    use mail_domain::Hue;
    match hue {
        Hue::Red => "Red",
        Hue::Orange => "Orange",
        Hue::Yellow => "Yellow",
        Hue::Green => "Green",
        Hue::Blue => "Blue",
        Hue::Purple => "Purple",
        Hue::Gray => "Gray",
    }
}

/// The word for the side of an action a conversation is offered.
pub fn offered(say: mail_client::conversation::Say) -> &'static str {
    use mail_client::conversation::Say;
    match say {
        Say::Archive => ARCHIVE,
        Say::MoveToInbox => MOVE_TO_INBOX,
        Say::NotSpam => NOT_SPAM,
        Say::Spam => MARK_AS_SPAM,
        Say::Trash => MOVE_TO_TRASH,
        Say::DeleteForever => DELETE_FOREVER,
        Say::MarkRead => MARK_AS_READ,
        Say::MarkUnread => MARK_AS_UNREAD,
        Say::Star => STAR,
        Say::Unstar => UNSTAR,
        Say::Pin => PIN,
        Say::Unpin => UNPIN,
        Say::Mute => MUTE,
        Say::Unmute => UNMUTE,
        Say::Snooze => SNOOZE,
        Say::Unsnooze => UNSNOOZE,
        Say::Remind => REMIND,
        Say::StopReminding => STOP_REMINDING,
        Say::Label => LABEL,
        Say::Reply => REPLY,
        Say::ReplyAll => REPLY_ALL,
        Say::Forward => FORWARD,
    }
}

/// One action of a rule, as the rules list and the rule's editor both say it: the conversation
/// menus' own words for the actions a menu has, and a label's or a folder's name quoted.
pub fn rule_action(action: &mail_domain::RuleAction) -> String {
    use mail_domain::RuleAction;
    match action {
        RuleAction::Label(name) => format!("Label “{name}”"),
        RuleAction::File(path) => format!("Move to “{path}”"),
        RuleAction::Archive => ARCHIVE.to_owned(),
        RuleAction::Trash => MOVE_TO_TRASH.to_owned(),
        RuleAction::Spam => MARK_AS_SPAM.to_owned(),
        RuleAction::MarkRead => MARK_AS_READ.to_owned(),
        RuleAction::Star => STAR.to_owned(),
        RuleAction::MarkUnread => MARK_AS_UNREAD.to_owned(),
        RuleAction::Notify => RULE_NOTIFY.to_owned(),
        RuleAction::Forward(to) => format!("Forward to {to}"),
        RuleAction::Reply(template) => format!("Reply with “{template}”"),
    }
}

// The rule actions no conversation menu has, and the pickers that finish two of them.
pub const RULE_NOTIFY: &str = "Notify Me";
pub const RULE_FORWARD: &str = "Forward to…";
pub const RULE_REPLY: &str = "Reply with Template…";
pub const RULE_SENDS_HELP: &str = "As mail arrives, while Mailo runs";
pub const FORWARD_TO: &str = "Forward to";
pub const AN_ADDRESS: &str = "Type an address";
pub const REPLY_WITH: &str = "Reply with";
pub const FIND_A_TEMPLATE: &str = "Find a template";
/// The row of the forward picker that takes what was typed.
pub fn forward_to(address: &str) -> String {
    format!("Forward to {address}")
}
// Mail's Apply Rules, on the messages picked.
pub const APPLY_RULES: &str = "Apply Rules";

// ⌘K's commands for the contact book, which has no page of its own.
pub const IMPORT_CONTACTS: &str = "Import Contacts…";
pub const EXPORT_CONTACTS: &str = "Export Contacts…";

// A rule's right-click menu in Settings › Rules.
pub const MOVE_UP: &str = "Move Up";
pub const MOVE_DOWN: &str = "Move Down";
pub const DELETE_RULE: &str = "Delete Rule";

// The sender's name in the reader, right-clicked.
pub const EDIT_CONTACT: &str = "Edit Contact…";
/// The sender's name in the reader, or a row's menu: make the sender a VIP, or no longer one.
pub const ADD_TO_VIPS: &str = "Add to VIPs";
pub const REMOVE_FROM_VIPS: &str = "Remove from VIPs";

// Words selected in a message, right-clicked: they go on the clipboard.
pub const COPY: &str = "Copy";

// A label's ⋯ in the sidebar, and its editor's first colour.
pub const EDIT_LABEL: &str = "Edit Label…";
pub const NO_COLOUR: &str = "None";
pub const ADD_TO_CONTACTS: &str = "Add to Contacts";
pub const SHOW_CONTACT_CARD: &str = "Show Contact Card";
/// ⌘K's way to Settings › Contacts.
pub const OPEN_CONTACTS: &str = "Contacts";

// A link's menu, at the pointer, in the Mac's order.
pub const OPEN_LINK: &str = "Open Link";
/// Where the link goes, in Peek, before going there.
pub const PREVIEW_LINK: &str = "Preview Link";
pub const COPY_LINK: &str = "Copy Link";

// An account tile's right-click menu at the top of the sidebar.
pub const SIGN_IN_AGAIN: &str = "Sign In Again…";
pub const REMOVE_FROM_SPACE: &str = "Remove from This Space";
pub const ACCOUNT_SETTINGS: &str = "Account Settings…";

/// A key's or certificate's menu in Settings (`pgp::actions`).
pub const COPY_PUBLIC_KEY: &str = "Copy Public Key";
pub const SAVE_PUBLIC_KEY: &str = "Save Public Key\u{2026}";
pub const EXPORT_SECRET_KEY: &str = "Export Secret Key\u{2026}";
pub const MARK_VERIFIED: &str = "Mark as Verified";
/// What Mark as Verified and Trust ask first, as their tooltip.
pub const CHECK_THE_FINGERPRINT: &str = "Check the fingerprint first";
pub const COPY_CERTIFICATE: &str = "Copy Certificate";
pub const SAVE_CERTIFICATE: &str = "Save Certificate\u{2026}";
pub const TRUST: &str = "Trust";
pub const DONT_TRUST: &str = "Don\u{2019}t Trust";

/// The composer's Signature row: the choice that sends the message unsigned.
pub const NO_SIGNATURE: &str = "None";

// The composer's Picture Size: how large the pictures a message carries are sent.
pub const PICTURES_SMALL: &str = "Small";
pub const PICTURES_MEDIUM: &str = "Medium";
pub const PICTURES_LARGE: &str = "Large";
pub const PICTURES_ACTUAL: &str = "Actual Size";

// The composer's format bar: the text sizes, Apple Mail's and Gmail's four.
pub const SIZE_SMALL: &str = "Small";
pub const SIZE_NORMAL: &str = "Normal";
pub const SIZE_LARGE: &str = "Large";
pub const SIZE_HUGE: &str = "Huge";

// The format bar's Paragraph menu, in Apple Mail's words.
pub const ALIGN_LEFT: &str = "Align Left";
pub const ALIGN_CENTER: &str = "Center";
pub const ALIGN_RIGHT: &str = "Align Right";
pub const INDENT: &str = "Increase Indentation";
pub const OUTDENT: &str = "Decrease Indentation";
pub const QUOTE_MORE: &str = "Increase Quote Level";
pub const QUOTE_LESS: &str = "Decrease Quote Level";

// The format bar's colours: none, then the palette.
pub const NO_TEXT_COLOUR: &str = "No Colour";
pub const GREY: &str = "Grey";
pub const RED: &str = "Red";
pub const ORANGE: &str = "Orange";
pub const GREEN: &str = "Green";
pub const BLUE: &str = "Blue";
pub const PURPLE: &str = "Purple";

/// What Sort By calls a key.
pub fn sort_key(property: mail_domain::Property) -> &'static str {
    use mail_domain::Property;
    match property {
        Property::Date => "Date",
        Property::From | Property::Sender => "Sender",
        Property::Subject => "Subject",
        Property::Size => "Size",
        Property::Attachments => "Attachments",
        Property::Pin => "Pinned",
        Property::Unread => "Unread",
        Property::Starred => "Starred",
    }
}

/// The two ways a key runs, ascending first, in the words the key reads in: Oldest First and
/// Newest First, not Ascending and Descending.
pub fn sort_dirs(property: mail_domain::Property) -> [&'static str; 2] {
    use mail_domain::Property;
    match property {
        Property::Date => ["Oldest First", "Newest First"],
        Property::From | Property::Sender | Property::Subject => ["A to Z", "Z to A"],
        Property::Size => ["Smallest First", "Largest First"],
        Property::Attachments => ["Without First", "With First"],
        Property::Pin => ["Pinned Last", "Pinned First"],
        Property::Unread => ["Unread Last", "Unread First"],
        Property::Starred => ["Starred Last", "Starred First"],
    }
}
