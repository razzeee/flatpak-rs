//! Closed, host-only fault controls, captured once by the installation owner.
use super::super::Result;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub(in crate::installation) struct Faults {
    phase: String,
    fail: Option<String>,
    stop: Option<String>,
    errno: i32,
    remaining: AtomicU32,
    stopped: AtomicBool,
    delay_ms: u64,
    delayed: AtomicBool,
    invalid: bool,
}

impl Faults {
    pub fn from_environment() -> Self {
        let phase = std::env::var("FLATPAK_RS_UPDATE_TEST_PHASE").unwrap_or("apply".into());
        let fail = std::env::var("FLATPAK_RS_UPDATE_FAIL_AT").ok();
        let stop = std::env::var("FLATPAK_RS_UPDATE_STOP_AFTER").ok();
        let kind = std::env::var("FLATPAK_RS_UPDATE_FAIL_KIND").unwrap_or("EIO".into());
        let count = std::env::var("FLATPAK_RS_UPDATE_FAIL_COUNT")
            .unwrap_or("1".into())
            .parse::<u32>();
        let delay = std::env::var("FLATPAK_RS_UPDATE_DELAY_MS")
            .unwrap_or("0".into())
            .parse::<u64>();
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
            || stop.as_ref().is_some_and(|v| !stops.contains(&v.as_str()))
            || !count.as_ref().is_ok_and(|n| (1..=100).contains(n))
            || !delay.as_ref().is_ok_and(|n| *n <= 5000)
            || (delay.as_ref().is_ok_and(|n| *n > 0) && (phase != "apply" || stop.is_some()));
        Self {
            phase,
            fail,
            stop,
            errno: if kind == "ENOSPC" {
                libc::ENOSPC
            } else {
                libc::EIO
            },
            remaining: AtomicU32::new(count.unwrap_or(0)),
            stopped: AtomicBool::new(false),
            delay_ms: delay.unwrap_or(0),
            delayed: AtomicBool::new(false),
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
        if self.phase == phase && self.fail.as_deref() == Some(site) && self.take_failure() {
            eprintln!("update injected {phase} failure at {site}");
            return Err(std::io::Error::from_raw_os_error(self.errno).into());
        }
        Ok(())
    }

    fn take_failure(&self) -> bool {
        let mut remaining = self.remaining.load(Ordering::Relaxed);
        while remaining > 0 {
            match self.remaining.compare_exchange_weak(
                remaining,
                remaining - 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(current) => remaining = current,
            }
        }
        false
    }

    pub fn checkpoint(&self, phase: &str, name: &str) {
        if phase == "apply"
            && name == "prepared"
            && self.delay_ms > 0
            && !self.delayed.swap(true, Ordering::Relaxed)
        {
            eprintln!("update delay apply prepared");
            std::thread::sleep(std::time::Duration::from_millis(self.delay_ms));
        }
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
