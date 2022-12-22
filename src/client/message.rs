use std::sync::Arc;
use workflow_core::channel::*;
use super::error::Error;

/// Internal control message signaling WebSocket state
/// change, WebSocket shutdown and other custom events.
#[derive(Debug,Clone,PartialEq,Eq,Hash)]
pub enum Ctl {
	/// Connection is opened
	Open,
	/// Connection is closed
	Closed,
	/// Dispatched when user invokes WebSocket::disconnect()
	Shutdown,
	/// Ctl reserved for RPC channels
	RpcCtl(u32),
	/// For use by external clients that need to send
	/// themselves a custom shutdown signal
	Custom(u32),
}

/// The enum containing a WebSocket or an internal control 
/// [`Ctl`] message. This enum defines the message type.
#[derive(Debug,Clone,PartialEq,Eq,Hash)]
pub enum Message {
	/// Text message
	Text(String),
	/// Binary message
	Binary(Vec<u8>),
	/// Internal control message represented by [`Ctl`]
	Ctl(Ctl)
}

impl Message {
	pub fn is_ctl(&self) -> bool {
		match self {
			Message::Ctl(_) => true,
			_ => false
		}
	}
}

impl From<Message> for Vec<u8> {
	fn from( msg: Message ) -> Self {
		match msg {
			Message::Text(string) => string.into(),
			Message::Binary(vec) => vec,
			Message::Ctl(ctl) => {
				panic!( "WebSocket - From<Message> for Vec<u8>: unsupported 'Ctl' message type: {:?}", ctl );
			}
		}
	}
}

impl From<Vec<u8>> for Message {
	fn from(vec: Vec<u8>) -> Self {
		Message::Binary(vec)
	}
}

impl From<String> for Message {
	fn from(s: String) -> Self {
		Message::Text(s)
	}
}

impl From<&str> for Message {
	fn from(s: &str) -> Self {
		Message::Text(s.to_string())
	}
}

impl AsRef<[u8]> for Message {
	fn as_ref( &self ) -> &[u8] {
		match self {
			Message::Text(string) => string.as_ref(),
			Message::Binary(vec) => vec.as_ref(),
			Message::Ctl(ctl) => {
				panic!( "WebSocket - AsRef<[u8]> for Message: unsupported 'Ctl' message type: {:?}", ctl );
			}
		}
	}
}

/// Wrapper for a WebSocket message being dispatched to the server.
/// This enum represents a message `Post` or `WithAck` type that
/// contains a callback that is invoked on successful message 
/// handoff to the underlying interface (a browser WebSocket interface
/// of tokio WebSocket interface))
#[derive(Clone)]
pub enum DispatchMessage {
	Post(Message),
	WithAck(Message, Sender<Result<Arc<()>,Arc<Error>>>),
	DispatcherShutdown,
}

impl DispatchMessage {
	pub fn is_ctl(&self) -> bool {
		match self {
			DispatchMessage::DispatcherShutdown => true,
			_ => false
		}
	}
}