//! Knowing when you have walked away.
//!
//! What idle must *not* do is remove the ring. It stops the animation and
//! leaves it: walking back to the machine and looking at which window has
//! focus, before touching anything, is the case this exists for.
//!
//! So the default is to freeze, not to hide. Freezing costs the same as hiding
//! — no frames either way — and keeps the one thing the program is for.
//!
//! `ext_idle_notify_v1` is the compositor asking us; there is no polling here
//! and no input monitoring, which also means no need to see what you type.

use wayland_client::globals::GlobalList;
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::{self, ExtIdleNotificationV1},
    ext_idle_notifier_v1::ExtIdleNotifierV1,
};

use super::Nimbus;

/// The notifier and a seat to watch, if this compositor has them.
pub struct Manager {
    notifier: Option<ExtIdleNotifierV1>,
    seat: Option<WlSeat>,
    /// The live subscription. Replaced whenever the threshold changes, because
    /// the timeout is fixed when the notification is created.
    notification: Option<ExtIdleNotificationV1>,
}

impl Manager {
    pub fn bind(globals: &GlobalList, qh: &QueueHandle<Nimbus>) -> Self {
        // Neither is an error to be missing. A compositor without idle
        // notification simply never goes idle, which is the same as the
        // default threshold of 0.
        let notifier = globals.bind::<ExtIdleNotifierV1, _, _>(qh, 1..=1, ()).ok();
        let seat = globals.bind::<WlSeat, _, _>(qh, 1..=9, ()).ok();
        Manager { notifier, seat, notification: None }
    }

    pub fn is_available(&self) -> bool {
        self.notifier.is_some() && self.seat.is_some()
    }

    /// Watch for `threshold` seconds of no input, or stop watching at 0.
    ///
    /// Called again whenever the setting changes: the timeout is baked into the
    /// notification object, so a new threshold means a new object.
    pub fn watch(&mut self, qh: &QueueHandle<Nimbus>, threshold: f64) {
        if let Some(old) = self.notification.take() {
            old.destroy();
        }
        let (Some(notifier), Some(seat)) = (&self.notifier, &self.seat) else {
            return;
        };
        if threshold <= 0.0 {
            return;
        }
        // Clamped so a fat-fingered 0.001 does not ask the compositor to tell
        // us about every gap between keystrokes.
        let ms = (threshold * 1000.0).clamp(1000.0, f64::from(u32::MAX)) as u32;
        self.notification = Some(notifier.get_idle_notification(ms, seat, qh, ()));
    }
}

// wl_seat's events are capabilities and a name; we want neither. The seat is
// bound only so the idle notification has something to hang off.
impl Dispatch<WlSeat, ()> for Nimbus {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtIdleNotifierV1, ()> for Nimbus {
    fn event(
        _: &mut Self,
        _: &ExtIdleNotifierV1,
        _: <ExtIdleNotifierV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtIdleNotificationV1, ()> for Nimbus {
    fn event(
        state: &mut Self,
        _: &ExtIdleNotificationV1,
        event: <ExtIdleNotificationV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => state.set_idle(true),
            ext_idle_notification_v1::Event::Resumed => state.set_idle(false),
            _ => {}
        }
    }
}
