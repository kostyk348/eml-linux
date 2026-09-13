//! emlcore — shared substrate for the EML-native Linux stack.
//!
//! One format everywhere: an RFC 822 / MIME document (`.eml`) is the atomic
//! object for services, logs, packages, IPC and configuration.
//!
//! * [`record`] — an ordered RFC 822 record (headers + raw body).
//! * [`chain`]  — a SHA-256 hash-chain over events (tamper-evidence).
//! * [`spool`]  — an append-only directory of event `.eml` files (bus/log).
//! * [`unit`]   — a service definition parsed from an `.eml` container.
//!
//! Wire compatibility follows `mime-os/docs/FORMAT.md` (EML-IPC): headers
//! `From` / `To` / `X-EMLBox-Msg` / `Content-Type`, filename `<seq>.<id>.msg.eml`.

pub mod chain;
pub mod record;
pub mod spool;
pub mod time;
pub mod unit;

pub use chain::{link_hash, GENESIS};
pub use record::Record;
pub use spool::{ChainStatus, Event, Spool};
pub use unit::{RestartPolicy, Unit};
