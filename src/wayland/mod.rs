use std::io;

use smithay_client_toolkit::reexports::client::{DispatchError, EventQueue};
use wayland_backend::client::{ReadEventsGuard, WaylandError};

use crate::State;

pub mod protocols;

/// Trait for dispatching Wayland events through [`funq`].
#[funq::callbacks(State)]
pub trait WaylandDispatch {
    /// Dispatch pending Wayland events.
    ///
    /// This is called whenever the Wayland socket has data available for
    /// reading.
    fn wayland_dispatch(&mut self);
}

impl WaylandDispatch for State {
    fn wayland_dispatch(&mut self) {
        let mut queue = self.wayland_queue.take().unwrap();

        if let Err(err) = self.wayland_dispatch_internal(&mut queue) {
            eprintln!("wayland dispatch failed: {err}");
        }

        self.wayland_queue = Some(queue);
    }
}

impl State {
    fn wayland_dispatch_internal(
        &mut self,
        queue: &mut EventQueue<Self>,
    ) -> Result<(), WaylandError> {
        // Try to read from the socket.
        let guard = queue.prepare_read();
        if let Some(Err(WaylandError::Io(err))) = guard.map(ReadEventsGuard::read) {
            if err.kind() != io::ErrorKind::WouldBlock {
                return Err(WaylandError::Io(err));
            }
        }

        // Dispatch all non-blocking Wayland events.
        loop {
            match queue.dispatch_pending(self) {
                Ok(0) => break,
                Ok(_) => (),
                Err(DispatchError::Backend(err)) => return Err(err),
                Err(DispatchError::BadMessage { .. }) => (),
            }
        }

        // Flush all responses to the compositor.
        if let Err(WaylandError::Io(err)) = queue.flush() {
            if err.kind() != io::ErrorKind::WouldBlock {
                return Err(WaylandError::Io(err));
            }
        }

        Ok(())
    }
}
