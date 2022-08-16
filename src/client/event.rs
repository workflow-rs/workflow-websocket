#[derive(Clone,Debug,PartialEq,Eq)]
pub enum Event {
	Open,
	// Error,
	// Close,
	// Closing,
	Closed(CloseEvent),
	// WebSocketError( WsErr )
}

#[derive(Clone,Debug,PartialEq,Eq)]
pub struct CloseEvent {
	pub code: u16,
	pub reason: String,
	pub was_clean: bool,
}

