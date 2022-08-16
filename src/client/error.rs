use async_std::channel::{SendError,TrySendError};
use wasm_bindgen::prelude::*;
use std::sync::PoisonError;
use thiserror::Error;

use super::message::DispatchMessage;

#[derive(Error, Debug)]
pub enum Error {

    #[error("JsValue {0:?}")]
    JsValue(JsValue),
    #[error("PoisonError")]
    PoisonError,
    
    #[error("InvalidState")]
    InvalidState(u16),
    #[error("DataEncoding")]
    DataEncoding,
    #[error("DataType")]
    DataType,
    #[error("WebSocket is already connected")]
    AlreadyConnected,
    #[error("WebSocket is not connected")]
    NotConnected,
    
    
    #[error("Dispatch channel send error")]
    DispatchChannelSend, //(SendError<DispatchMessage>),
    #[error("Dispatch channel try_send error")]
    DispatchChannelTrySend, //(TrySendError<DispatchMessage>)
    
    #[error("Receive channel error")]
    ReceiveChannel,
    
    #[error("Connect channel error")]
    ConnectChannel,
    
    #[cfg(not(target_arch = "wasm32"))]
    #[error("WebSocket error: {0}")]
    Tungstenite(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("Unable to send ctl to receiver: {0}")]
    ReceiverCtlSend(#[from] SendError<super::message::Message>),


}


impl From<JsValue> for Error {
    fn from(error: JsValue) -> Error {
        Error::JsValue(error)
    }
}

impl<T> From<PoisonError<T>> for Error {
    fn from(_: PoisonError<T>) -> Error {
        Error::PoisonError
    }
}

// impl<T> From<SendError<T>> for Error {
//     fn from(_error: SendError<T>) -> Error {
//         Error::DispatchChannelSendError //(error)
//     }
// }

impl From<SendError<DispatchMessage>> for Error {
    fn from(_error: SendError<DispatchMessage>) -> Error {
        Error::DispatchChannelSend //(error)
    }
}

impl From<TrySendError<DispatchMessage>> for Error {
    fn from(_error: TrySendError<DispatchMessage>) -> Error {
        Error::DispatchChannelTrySend //(error)
    }
}

