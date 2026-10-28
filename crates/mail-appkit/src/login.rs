//! The login item, as macOS 13 and later keep one: `SMAppService`, with the launch agent a
//! property list in the app's own bundle (`Contents/Library/LaunchAgents/`, put there by
//! `packaging/macos/bundle.sh`). Registering it is what makes launchd start the agent at each login
//! and lists mailo under System Settings › General › Login Items, where the person can switch it
//! off. The `unsafe` is the framework's calls, every one of which objc2 marks unsafe.
#![allow(unsafe_code)]

use objc2::runtime::AnyClass;
use objc2_foundation::NSString;
use objc2_service_management::{SMAppService, SMAppServiceStatus};

/// Where an agent stands after it was asked to start at login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registered {
    /// launchd starts it at each login (and has started it now).
    Enabled,
    /// It is registered, and waits for the person to allow it in Login Items.
    NeedsApproval,
}

/// Why `SMAppService` could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    /// Before macOS 13 there is no `SMAppService`; the caller keeps the agent another way.
    NoService,
    /// This copy of mailo has no such property list in its bundle: not run from mailo.app, or a
    /// bundle made before it carried one.
    NotInBundle,
    /// The system refused, in its words.
    Refused(String),
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unavailable::NoService => f.write_str("this macOS has no SMAppService"),
            Unavailable::NotInBundle => {
                f.write_str("the launch agent is not in this application's bundle")
            }
            Unavailable::Refused(why) => f.write_str(why),
        }
    }
}

/// The agent whose property list is `plist` (a file name in the bundle's
/// `Contents/Library/LaunchAgents/`), or why there is none to ask.
fn agent(plist: &str) -> Result<objc2::rc::Retained<SMAppService>, Unavailable> {
    // The framework is linked on every macOS this builds for; the class is there from 13 on.
    if AnyClass::get(c"SMAppService").is_none() {
        return Err(Unavailable::NoService);
    }
    let service = unsafe { SMAppService::agentServiceWithPlistName(&NSString::from_str(plist)) };
    match unsafe { service.status() } {
        SMAppServiceStatus::NotFound => Err(Unavailable::NotInBundle),
        _ => Ok(service),
    }
}

/// Have launchd start the agent `plist` names at each login. Already registered is done.
pub fn register_agent(plist: &str) -> Result<Registered, Unavailable> {
    let service = agent(plist)?;
    if unsafe { service.status() } != SMAppServiceStatus::Enabled {
        unsafe { service.registerAndReturnError() }
            .map_err(|error| Unavailable::Refused(error.localizedDescription().to_string()))?;
    }
    Ok(match unsafe { service.status() } {
        SMAppServiceStatus::RequiresApproval => Registered::NeedsApproval,
        _ => Registered::Enabled,
    })
}

/// Stop launchd starting the agent `plist` names, and stop the one running. Never registered is
/// done.
pub fn unregister_agent(plist: &str) -> Result<(), Unavailable> {
    let service = agent(plist)?;
    if unsafe { service.status() } == SMAppServiceStatus::NotRegistered {
        return Ok(());
    }
    unsafe { service.unregisterAndReturnError() }
        .map_err(|error| Unavailable::Refused(error.localizedDescription().to_string()))
}
