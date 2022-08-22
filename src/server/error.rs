use thiserror::Error;
use tokio::sync::mpsc::error::SendError;

#[derive(Debug, Error)]
pub enum Error {
    /// Indicates that no messages have been received
    /// within the specified timeout period
    #[error("Connection timeout")]
    ConnectionTimeout,

    /// Indicates that the data received is not a 
    /// valid websocket message
    #[error("Malformed handshake message")]
    MalformedHandshake,

    /// Indicates handler negotiation failure
    /// This error code is reserved for structs
    /// implementing WebSocket handler.
    #[error("Negotiation failure")]
    NegotiationFailure,
    
    /// Indicates handler negotiation failure
    /// with a specific reason
    /// This error code is reserved for structs
    /// implementing WebSocket handler.
    #[error("Negotiation failure: {0}")]
    NegotiationFailureWithReason(String),
    
    /// Error sending response via the 
    /// tokio mspc response channel
    #[error("Response channel send error {0:?}")]
    ResponseChannelError(#[from] SendError<tungstenite::Message>),

    /// WebSocket error produced by the underlying
    /// Tungstenite WebSocket crate
    #[error("WebSocket error: {0}")]
    WebSocketError(#[from] tungstenite::Error),
    
    /// Connection terminated absormally
    #[error("Connection closed abnormally")]
    ConnectionClosed,
}