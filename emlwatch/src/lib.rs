//! emlwatch — map inotify filesystem events into `.eml` records.

use inotify::{EventMask, WatchMask};

/// Parse a comma list (`create,modify,delete,...`) into a watch mask.
pub fn parse_watch_mask(spec: &str) -> WatchMask {
    let s = spec.trim();
    if s.is_empty() || s == "all" {
        return WatchMask::ALL_EVENTS;
    }
    let mut m = WatchMask::empty();
    for part in s.split(',') {
        match part.trim() {
            "create" => m |= WatchMask::CREATE,
            "modify" => m |= WatchMask::MODIFY,
            "delete" => m |= WatchMask::DELETE,
            "close" => m |= WatchMask::CLOSE_WRITE,
            "move" => m |= WatchMask::MOVED_TO | WatchMask::MOVED_FROM,
            "attrib" => m |= WatchMask::ATTRIB,
            _ => {}
        }
    }
    if m.is_empty() {
        WatchMask::ALL_EVENTS
    } else {
        m
    }
}

/// A short human label for the primary action of an event.
pub fn event_kind(mask: EventMask) -> &'static str {
    if mask.contains(EventMask::CREATE) {
        "CREATE"
    } else if mask.contains(EventMask::DELETE) {
        "DELETE"
    } else if mask.contains(EventMask::MOVED_TO) {
        "MOVED_TO"
    } else if mask.contains(EventMask::MOVED_FROM) {
        "MOVED_FROM"
    } else if mask.contains(EventMask::MODIFY) {
        "MODIFY"
    } else if mask.contains(EventMask::CLOSE_WRITE) {
        "CLOSE_WRITE"
    } else if mask.contains(EventMask::ATTRIB) {
        "ATTRIB"
    } else {
        "OTHER"
    }
}

/// Space-separated set of all flags present in `mask` (for the event body).
pub fn describe(mask: EventMask) -> String {
    let mut parts = Vec::new();
    for (flag, name) in [
        (EventMask::CREATE, "CREATE"),
        (EventMask::MODIFY, "MODIFY"),
        (EventMask::DELETE, "DELETE"),
        (EventMask::MOVED_TO, "MOVED_TO"),
        (EventMask::MOVED_FROM, "MOVED_FROM"),
        (EventMask::CLOSE_WRITE, "CLOSE_WRITE"),
        (EventMask::ATTRIB, "ATTRIB"),
        (EventMask::ISDIR, "ISDIR"),
    ] {
        if mask.contains(flag) {
            parts.push(name);
        }
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_priority() {
        assert_eq!(event_kind(EventMask::CREATE), "CREATE");
        assert_eq!(event_kind(EventMask::MODIFY), "MODIFY");
        assert_eq!(event_kind(EventMask::DELETE), "DELETE");
        assert_eq!(
            event_kind(EventMask::CREATE | EventMask::ISDIR),
            "CREATE"
        );
    }

    #[test]
    fn describe_lists_flags() {
        let d = describe(EventMask::CREATE | EventMask::ISDIR);
        assert!(d.contains("CREATE"));
        assert!(d.contains("ISDIR"));
    }

    #[test]
    fn mask_parsing() {
        let m = parse_watch_mask("create,delete");
        assert!(m.contains(WatchMask::CREATE));
        assert!(m.contains(WatchMask::DELETE));
        assert!(!m.contains(WatchMask::MODIFY));
    }
}
