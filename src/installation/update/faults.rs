//! Closed, host-only fault controls, captured once by the installation owner.
use super::super::Result;
use std::sync::atomic::{AtomicBool, Ordering};

pub(in crate::installation) struct Faults {
    phase: String,
    fail: Option<String>,
    stop: Option<String>,
    errno: i32,
    used: AtomicBool,
    stopped: AtomicBool,
    invalid: bool,
}

impl Faults {
    pub fn from_environment() -> Self {
        let phase = std::env::var("FLATPAK_RS_UPDATE_TEST_PHASE").unwrap_or("apply".into());
        let fail = std::env::var("FLATPAK_RS_UPDATE_FAIL_AT").ok();
        let stop = std::env::var("FLATPAK_RS_UPDATE_STOP_AFTER").ok();
        let kind = std::env::var("FLATPAK_RS_UPDATE_FAIL_KIND").unwrap_or("EIO".into());
        let sites = [
            "write-selection",
            "sync-selection",
            "sync-payload",
            "sync-retained",
            "sync-intent",
            "exchange",
            "exchange-result",
            "sync-installed-parent",
            "sync-slot-parent",
            "mark-garbage",
            "sync-active-parent",
            "sync-garbage-parent",
            "delete-obsolete",
            "sync-cleanup",
        ];
        let stops = [
            "imported",
            "prepared",
            "retained",
            "intent",
            "exchanged",
            "parents-synced",
            "committed",
            "cleanup-started",
            "cleanup-partial",
        ];
        let invalid = !["apply", "recovery"].contains(&phase.as_str())
            || !["EIO", "ENOSPC"].contains(&kind.as_str())
            || fail.as_ref().is_some_and(|v| !sites.contains(&v.as_str()))
            || stop.as_ref().is_some_and(|v| !stops.contains(&v.as_str()));
        Self {
            phase,
            fail,
            stop,
            errno: if kind == "ENOSPC" {
                libc::ENOSPC
            } else {
                libc::EIO
            },
            used: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            invalid,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.invalid {
            Err("invalid host local-update fault configuration".into())
        } else {
            Ok(())
        }
    }

    pub fn check(&self, phase: &str, site: &str) -> Result<()> {
        self.validate()?;
        if self.phase == phase
            && self.fail.as_deref() == Some(site)
            && !self.used.swap(true, Ordering::Relaxed)
        {
            eprintln!("update injected {phase} failure at {site}");
            return Err(std::io::Error::from_raw_os_error(self.errno).into());
        }
        Ok(())
    }

    pub fn checkpoint(&self, phase: &str, name: &str) {
        if self.phase == phase
            && self.stop.as_deref() == Some(name)
            && !self.stopped.swap(true, Ordering::Relaxed)
        {
            eprintln!("update checkpoint {phase} {name}");
            // SAFETY: host-configured process stop for the external acceptance harness.
            unsafe {
                libc::raise(libc::SIGSTOP);
            }
        }
    }
}
