//! Power assertions owned by one authenticated remote display session.

use anyhow::{bail, Result};
use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOPMAssertionCreateWithName(
        assertion_type: CFStringRef,
        level: u32,
        name: CFStringRef,
        assertion_id: *mut u32,
    ) -> i32;
    fn IOPMAssertionDeclareUserActivity(
        name: CFStringRef,
        user_type: u32,
        assertion_id: *mut u32,
    ) -> i32;
    fn IOPMAssertionRelease(assertion_id: u32) -> i32;
}

struct Assertion(u32);

impl Drop for Assertion {
    fn drop(&mut self) {
        // The activity assertion may already have expired. Releasing our own ID is safe
        // in either case, and never changes the user's configured sleep timers.
        unsafe {
            IOPMAssertionRelease(self.0);
        }
    }
}

/// Dropped on initialization failure, disconnect, reactivation or server shutdown.
pub(crate) struct DisplayPower {
    _awake: Assertion,
    _activity: Assertion,
}

impl DisplayPower {
    /// Call only after RDP authentication, before querying capturable displays.
    pub(crate) fn acquire() -> Result<Self> {
        let name = CFString::new("macrdp authenticated remote desktop");
        let assertion_type = CFString::new("PreventUserIdleDisplaySleep");
        let mut awake_id = 0;
        // kIOPMAssertionLevelOn = 255. All CF objects outlive their synchronous calls.
        let result = unsafe {
            IOPMAssertionCreateWithName(
                assertion_type.as_concrete_TypeRef(),
                255,
                name.as_concrete_TypeRef(),
                &mut awake_id,
            )
        };
        if result != 0 {
            bail!("Failed to keep the remote display awake (IOKit {result:#x})");
        }
        let awake = Assertion(awake_id);

        // PreventUserIdleDisplaySleep alone cannot wake an already sleeping display.
        // kIOPMUserActiveRemote = 1 requests framebuffer/GPU access for a remote user.
        let mut activity_id = 0;
        let result = unsafe {
            IOPMAssertionDeclareUserActivity(name.as_concrete_TypeRef(), 1, &mut activity_id)
        };
        if result != 0 {
            bail!("Failed to wake the remote display (IOKit {result:#x})");
        }

        Ok(Self {
            _awake: awake,
            _activity: Assertion(activity_id),
        })
    }
}
