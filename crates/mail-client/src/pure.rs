//! What a caller may name of `mail-core`: its words for mail (a document being edited, an undo, a
//! time as a person reads it) and what it does with no store and no server (parsing a search, the
//! files under the config directory). Everything that reads or writes the store, or asks a
//! server, is a call on [`Client`](crate::Client) instead, so a caller handed only a client has no
//! way to name a store.
//!
//! Each module stands for the `mail-core` module of the same name, and holds only what a caller
//! uses of it.

pub mod account {
    pub use mail_core::account::{
        Credentials, Mode, Outcome, RemoveError, Setup, SignInError, authorize, oauth_client,
    };
    pub mod direct {
        pub use mail_core::account::direct::{Profiles, WAIT, signs_in_directly};
    }
    pub mod draft {
        pub use mail_core::account::draft::{
            Action, CopyMark, Draft, FieldDraft, ListKey, Out, Sheet, acted, is_manual, problem_of,
            retries, shown,
        };
    }
}
pub mod attach {
    pub use mail_core::attach::{
        downloads_dir, human_size, opened_attachment, runs_when_opened, safe_name, save_opened,
        write_new,
    };
}
pub mod auth {
    pub use mail_core::auth::{Standing, sentence, standing};
}
pub mod bimi {
    pub use mail_core::bimi::{
        Cached, Lookup, Setting, SystemDns, anchors, brand_logo, cached, client_builder, domain_of,
        load,
    };
}
pub mod compose {
    pub use mail_core::compose::{ATTACHMENT_BUDGET, Leaves, SendError};
    pub mod addresses {
        pub use mail_core::compose::addresses::{join_addresses, parse_addresses};
    }
}
pub mod config {
    pub use mail_core::config::{
        cache_dir, config_dir, home_dir, json_damage, read_json, state_dir, write_json, write_text,
    };
}
pub mod contacts {
    pub use mail_core::contacts::{Contact, Imported, Origin};
    pub mod groups {
        pub use mail_core::contacts::groups::{Offer, person_of};
    }
}
pub mod discover {
    pub use mail_core::discover::{
        Failed, Found, Looked, claimed_by, known, settle, typed, typed_domain,
    };
    pub mod providers {
        pub use mail_core::discover::providers::set;
    }
}
pub mod rich_text {
    pub use mail_core::rich_text::{
        Action, Caret, Check, Doc, ImageRef, InputEvent, Item, Mark, Marks, Node, Object, Op,
        ParaKind, Person, Pos, Presence, Range, Run, SNIPPET, Session, Table, apply_all, catalog,
        filter, grapheme_len, joins_cc, missing_attachment, node_len, nodes_from_blocks,
        nodes_from_plain, runs_text, to_flowed, to_html, turn_into,
    };
}
pub mod export {
    pub use mail_core::export::{Exported, Target};
}
pub mod folder {
    pub use mail_core::folder::Refusal;
}
pub mod follow_up {
    pub use mail_core::follow_up::{CHOICES, due, returned_words};
}
pub mod import {
    pub use mail_core::import::{Destination, Imported, Source};
}
pub mod invite {
    pub use mail_core::invite::{Answered, repeats_words, say_when};
}
pub mod ipc {
    pub use mail_core::ipc::agent;
    pub mod watching {
        pub use mail_core::ipc::watching::running;
    }
}
pub mod mail {
    pub use mail_core::mail::{Clock, SystemClock};
}
pub mod message {
    pub use mail_core::message::{
        Bodies, FIRST_PAGE, FrameBody, Frames, INLINE_BYTES, Invited, Key, Looks, Offer, Opening,
        Protection, Receipt, Scheme, Sealed, Sent, Tried, ahead,
    };
}
pub mod notify {
    pub use mail_core::notify::{Notifier, Setting, load};
    #[cfg(feature = "desktop")]
    pub mod desktop {
        pub use mail_core::notify::desktop::Desktop;
    }
}
pub mod offline {
    pub use mail_core::offline::{FILE_NAME, Keep, damage, load, said, save};
}
pub mod password {
    pub use mail_core::password::Password;
}
pub mod pgp {
    pub use mail_core::pgp::{Ask, PgpError, Protected, WithSecret, no_passphrase};
    pub mod keys {
        pub use mail_core::pgp::keys::export_public;
    }
}
pub mod mailbox_filter {
    pub use mail_core::mailbox_filter::{for_role, pending_snooze, pin_op, role_of, unread_in};
}
pub mod attachment_image {
    pub use mail_core::attachment_image::{
        Kind, LARGE, PDF_PAGE, Page, Picture, SMALL, Unavailable, pdf_page, picture, sniff,
    };
}
pub mod html_document {
    pub use mail_core::html_document::{LABELS, Pictures, Printed, write_file_into};
}
pub mod profile {
    pub use mail_core::profile::ProfileId;
}
pub mod provider {
    pub use mail_core::provider::{Provider, provider, service_name};
    pub mod icon {
        pub use mail_core::provider::icon::{Loaded, missing, refresh};
    }
}
pub mod receipt {
    pub use mail_core::receipt::ReceiptState;
}
pub mod remote_image {
    pub use mail_core::remote_image::{Image, ImageFetcher};
}
pub mod rules {
    pub use mail_core::rules::ListedRule;
    pub mod manage {
        pub use mail_core::rules::manage::{
            ConditionError, Draft, SaveError, Step, read_condition,
        };
    }
    pub mod server {
        pub use mail_core::rules::server::{instant, vacation_for};
    }
}
pub mod schedule {
    pub use mail_core::schedule::{Daemon, Lost, Schedule, Timing, probe};
}
pub mod scope {
    pub use mail_core::scope::Scope;
}
pub mod search {
    // `run` reads what it is handed: a caller has only the client's (`Client::source`).
    pub use mail_core::search::{
        ActionHit, Affinity, Command, Highlight, PersonHit, Results, SenderStats, Source, Top,
        first, list_highlight, match_list, parse, run, search_list, snippet,
    };
}
pub mod server_search {
    pub use mail_core::server_search::{Automatic, load, searchable};
}
pub mod smime {
    pub use mail_core::smime::{Protected, SmimeError};
    pub mod certs {
        pub use mail_core::smime::certs::export;
    }
}
pub mod snooze {
    pub use mail_core::snooze::snooze_until;
}
pub mod sync {
    pub use mail_core::sync::Configured;
    pub mod due {
        pub use mail_core::sync::due::IDLE;
    }
    pub mod fresh {
        pub use mail_core::sync::fresh::{Freshness, Pace, of_link};
    }
    pub mod report {
        pub use mail_core::sync::report::Offer;
    }
}
pub mod transfer {
    pub use mail_core::transfer::{Format, IoCause, Refusal, Seen, expand_here, look, suggested};
}
pub mod trust {
    pub use mail_core::trust::{Destination, destination, spoof};
}
pub mod undo {
    pub use mail_core::undo::{Resend, Undo, UndoHandle, UndoStack, reverse_folder, said_of};
}
pub mod unsubscribe {
    pub use mail_core::unsubscribe::{Found, Outcome, client};
}
pub mod conversation {
    pub use mail_core::conversation::{
        Place, Say, Source, allowed_actions, default_places, find_place, folder_filter, folder_of,
        is_label_place, label_choices, mute_for_all, mute_op, offers, op_for, place_key,
        places_with, reply_target, saved_of, saved_place, says, says_for_all, scoped,
        scoped_unread_filter, step, turns_off, unread_filter,
    };
}
pub mod saved_search {
    pub use mail_core::saved_search::{Draft, Refusal, written};
}
pub mod wanted {
    pub use mail_core::wanted::{ask_first, claim};
}
pub mod when {
    pub use mail_core::when::{Clock, Stamp, listed, stamp, time_of_day, upcoming};
}
