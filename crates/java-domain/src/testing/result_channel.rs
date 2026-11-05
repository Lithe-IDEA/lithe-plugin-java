//! Lifecycle of the loopback channel that receives Java test results.
//!
//! The Java test runner connects back to a loopback port and streams its
//! results. Ported from the Swift `JavaTestResultServer` as a sans-IO state
//! machine: the driver owns the socket and the clock, feeds listener events
//! in, and executes the returned [`Action`]s. Every listener gets a
//! generation, so events from a listener that was already replaced are
//! ignored, and late connections to it are closed.
//!
//! TODO(host-api: API-29 build and test): a plugin backend cannot open
//! listening sockets today. The driver needs either a host-owned loopback
//! channel that forwards the runner connection to the backend, or a backend
//! `network.loopbackListen` permission. Until then the host keeps its listener
//! and this module only fixes the rules it must follow.

use std::time::{Duration, Instant};

/// How long the listener may take to report its port.
pub const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

/// Identifier of one listener start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Generation(u64);

/// Identifier the driver gives an accepted connection.
pub type ConnectionId = u64;

/// Something the listener of one generation reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListenerEvent {
    /// The listener is bound; `port` is `None` when the system assigned none.
    Ready {
        /// Assigned loopback port.
        port: Option<u16>,
    },
    /// The listener failed.
    Failed {
        /// System error description.
        message: String,
    },
    /// The listener was cancelled.
    Cancelled,
    /// The runner connected.
    Connection {
        /// Driver-assigned connection identifier.
        id: ConnectionId,
    },
    /// A tracked connection closed.
    ConnectionEnded {
        /// Driver-assigned connection identifier.
        id: ConnectionId,
    },
}

/// Why startup did not produce a port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultChannelError {
    /// The listener failed before it was ready.
    StartupFailed(String),
    /// The listener did not report a port in time.
    StartupTimedOut,
    /// Startup was abandoned by [`ResultChannel::stop`].
    Cancelled,
}

impl std::fmt::Display for ResultChannelError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StartupFailed(message) => write!(
                formatter,
                "Could not start the Java test result listener: {message}"
            ),
            Self::StartupTimedOut => {
                formatter.write_str("The Java test result listener did not become ready in time.")
            }
            Self::Cancelled => formatter.write_str("The Java test result listener was cancelled."),
        }
    }
}

impl std::error::Error for ResultChannelError {}

/// Work the driver performs on behalf of the state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Bind a new loopback listener on an ephemeral port for `generation`.
    StartListener {
        /// Generation the listener reports events for.
        generation: Generation,
    },
    /// Stop accepting connections on the current listener.
    CancelListener,
    /// Close one connection.
    CloseConnection {
        /// Connection to close.
        id: ConnectionId,
    },
    /// Resolve the pending [`ResultChannel::start`] call.
    CompleteStartup(Result<u16, ResultChannelError>),
}

#[derive(Debug)]
struct Pending {
    deadline: Instant,
}

/// Single-connection result channel state.
#[derive(Debug)]
pub struct ResultChannel {
    startup_timeout: Duration,
    generation: u64,
    listening: bool,
    pending: Option<Pending>,
    connections: Vec<ConnectionId>,
    port: Option<u16>,
}

impl Default for ResultChannel {
    fn default() -> Self {
        Self::new(DEFAULT_STARTUP_TIMEOUT)
    }
}

impl ResultChannel {
    /// Creates a stopped channel with the given startup timeout.
    pub fn new(startup_timeout: Duration) -> Self {
        Self {
            startup_timeout,
            generation: 0,
            listening: false,
            pending: None,
            connections: Vec::new(),
            port: None,
        }
    }

    /// Port of the ready listener.
    pub fn port(&self) -> Option<u16> {
        self.port
    }

    /// Connections currently tracked.
    pub fn connections(&self) -> &[ConnectionId] {
        &self.connections
    }

    /// Stops any previous listener and starts a new one at `now`.
    pub fn start(&mut self, now: Instant) -> (Generation, Vec<Action>) {
        let mut actions = self.stop();
        self.generation += 1;
        let generation = Generation(self.generation);
        self.listening = true;
        self.pending = Some(Pending {
            deadline: now + self.startup_timeout,
        });
        actions.push(Action::StartListener { generation });
        (generation, actions)
    }

    /// Cancels the listener, closes every connection and abandons startup.
    pub fn stop(&mut self) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.listening {
            self.listening = false;
            actions.push(Action::CancelListener);
        }
        actions.extend(
            self.connections
                .drain(..)
                .map(|id| Action::CloseConnection { id }),
        );
        self.port = None;
        if self.pending.take().is_some() {
            actions.push(Action::CompleteStartup(Err(ResultChannelError::Cancelled)));
        }
        actions
    }

    /// Applies one listener event.
    pub fn handle(&mut self, generation: Generation, event: ListenerEvent) -> Vec<Action> {
        if generation.0 != self.generation || !self.is_active() {
            return match event {
                ListenerEvent::Connection { id } => vec![Action::CloseConnection { id }],
                _ => Vec::new(),
            };
        }
        match event {
            ListenerEvent::Ready { port: Some(port) } => {
                self.port = Some(port);
                match self.pending.take() {
                    Some(_) => vec![Action::CompleteStartup(Ok(port))],
                    None => Vec::new(),
                }
            }
            ListenerEvent::Ready { port: None } => self.fail_startup(
                ResultChannelError::StartupFailed("No listening port was assigned.".into()),
            ),
            ListenerEvent::Failed { message } => {
                self.fail_startup(ResultChannelError::StartupFailed(message))
            }
            ListenerEvent::Cancelled => self.fail_startup(ResultChannelError::Cancelled),
            ListenerEvent::Connection { id } => {
                // The runner opens exactly one connection; refuse any other.
                let mut actions = Vec::new();
                if self.listening {
                    self.listening = false;
                    actions.push(Action::CancelListener);
                }
                self.connections.push(id);
                actions
            }
            ListenerEvent::ConnectionEnded { id } => {
                self.connections.retain(|tracked| *tracked != id);
                Vec::new()
            }
        }
    }

    /// Times out a startup whose deadline passed.
    pub fn poll(&mut self, now: Instant) -> Vec<Action> {
        match &self.pending {
            Some(pending) if now >= pending.deadline => {
                self.fail_startup(ResultChannelError::StartupTimedOut)
            }
            _ => Vec::new(),
        }
    }

    fn is_active(&self) -> bool {
        self.listening || self.pending.is_some() || !self.connections.is_empty()
    }

    fn fail_startup(&mut self, error: ResultChannelError) -> Vec<Action> {
        let Some(_) = self.pending.take() else {
            return Vec::new();
        };
        let mut actions = self.stop();
        actions.push(Action::CompleteStartup(Err(error)));
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started(channel: &mut ResultChannel, now: Instant) -> Generation {
        let (generation, actions) = channel.start(now);
        assert_eq!(actions, vec![Action::StartListener { generation }]);
        generation
    }

    #[test]
    fn start_returns_the_ready_port_and_stop_cancels_the_listener() {
        let now = Instant::now();
        let mut channel = ResultChannel::default();
        let generation = started(&mut channel, now);
        assert_eq!(
            channel.handle(generation, ListenerEvent::Ready { port: Some(43_128) }),
            vec![Action::CompleteStartup(Ok(43_128))]
        );
        assert_eq!(channel.port(), Some(43_128));
        assert_eq!(channel.stop(), vec![Action::CancelListener]);
        assert_eq!(channel.port(), None);
    }

    #[test]
    fn cancelling_startup_stops_the_pending_listener() {
        let now = Instant::now();
        let mut channel = ResultChannel::default();
        started(&mut channel, now);
        assert_eq!(
            channel.stop(),
            vec![
                Action::CancelListener,
                Action::CompleteStartup(Err(ResultChannelError::Cancelled))
            ]
        );
        assert!(channel.stop().is_empty());
    }

    #[test]
    fn startup_times_out_at_the_deadline_only() {
        let now = Instant::now();
        let mut channel = ResultChannel::new(Duration::from_millis(100));
        started(&mut channel, now);
        assert!(channel.poll(now + Duration::from_millis(99)).is_empty());
        let actions = channel.poll(now + Duration::from_millis(100));
        assert_eq!(
            actions,
            vec![
                Action::CancelListener,
                Action::CompleteStartup(Err(ResultChannelError::StartupTimedOut))
            ]
        );
        assert_eq!(
            ResultChannelError::StartupTimedOut.to_string(),
            "The Java test result listener did not become ready in time."
        );
    }

    #[test]
    fn listener_failures_resolve_startup_with_the_host_message() {
        let now = Instant::now();
        let mut channel = ResultChannel::default();
        let generation = started(&mut channel, now);
        let actions = channel.handle(generation, ListenerEvent::Ready { port: None });
        let Some(Action::CompleteStartup(Err(error))) = actions.last() else {
            panic!("startup should fail: {actions:?}");
        };
        assert_eq!(
            error.to_string(),
            "Could not start the Java test result listener: No listening port was assigned."
        );
    }

    #[test]
    fn events_from_a_replaced_listener_are_ignored_and_its_connections_closed() {
        let now = Instant::now();
        let mut channel = ResultChannel::default();
        let old = started(&mut channel, now);
        let (new, actions) = channel.start(now);
        assert_eq!(
            actions,
            vec![
                Action::CancelListener,
                Action::CompleteStartup(Err(ResultChannelError::Cancelled)),
                Action::StartListener { generation: new }
            ]
        );
        assert!(channel
            .handle(old, ListenerEvent::Ready { port: Some(1) })
            .is_empty());
        assert_eq!(
            channel.handle(old, ListenerEvent::Connection { id: 7 }),
            vec![Action::CloseConnection { id: 7 }]
        );
        assert_eq!(
            channel.handle(new, ListenerEvent::Ready { port: Some(2) }),
            vec![Action::CompleteStartup(Ok(2))]
        );
    }

    #[test]
    fn the_first_connection_closes_the_listener_to_further_runners() {
        let now = Instant::now();
        let mut channel = ResultChannel::default();
        let generation = started(&mut channel, now);
        channel.handle(generation, ListenerEvent::Ready { port: Some(9) });
        assert_eq!(
            channel.handle(generation, ListenerEvent::Connection { id: 1 }),
            vec![Action::CancelListener]
        );
        assert_eq!(channel.connections(), &[1]);
        assert!(channel
            .handle(generation, ListenerEvent::ConnectionEnded { id: 1 })
            .is_empty());
        assert!(channel.connections().is_empty());
        assert_eq!(
            channel.handle(generation, ListenerEvent::Connection { id: 2 }),
            vec![Action::CloseConnection { id: 2 }]
        );
    }

    #[test]
    fn stop_closes_open_connections() {
        let now = Instant::now();
        let mut channel = ResultChannel::default();
        let generation = started(&mut channel, now);
        channel.handle(generation, ListenerEvent::Ready { port: Some(9) });
        channel.handle(generation, ListenerEvent::Connection { id: 4 });
        assert_eq!(channel.stop(), vec![Action::CloseConnection { id: 4 }]);
    }
}
