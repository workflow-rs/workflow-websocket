use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(target_arch = "wasm32")] {
        mod wasm;
        pub use wasm::WebSocketInterface;
    } else {
        mod native;
        pub use native::WebSocketInterface;
    }
}

pub mod result;
pub mod error;
pub mod message;
pub mod event;
pub mod state;

pub use result::Result;
pub use error::Error;
pub use message::*;

pub use message::Message;

use std::sync::Arc;
use async_std::channel::{Receiver,Sender,unbounded};
use workflow_core::channel::oneshot;

#[derive(Clone)] 
pub struct WebSocket {
    client : Arc<WebSocketInterface>,
    pub sender_tx : Sender<DispatchMessage>,
    pub receiver_rx : Receiver<Message>,
    pub receiver_tx : Sender<Message>,
}

impl WebSocket {
    // pub fn new(url : &str, settings : Settings) -> Result<Arc<WebSocket>> {
    pub fn new(url : &str) -> Result<WebSocket> {

        let (receiver_tx, receiver_rx) = unbounded::<Message>();
        let (sender_tx, sender_tx_rx) = { 
            let tx_rx = unbounded::<DispatchMessage>();
            let tx = tx_rx.0.clone();
            (tx, tx_rx)
        };

        let client = Arc::new(WebSocketInterface::new(
            url,
            receiver_tx.clone(),
            sender_tx_rx
        )?);

        let websocket = WebSocket {
            client,
            sender_tx,
            receiver_rx,
            receiver_tx,
        };

        Ok(websocket)
    }

    pub async fn connect(&self, block_until_connected : bool) -> Result<()> {
        Ok(self.client.connect(block_until_connected).await?)
    }

    pub async fn disconnect(&self) -> Result<()> {
        Ok(self.client.disconnect().await?)
    }

    pub async fn post(&self, message: Message) -> Result<&Self> {
        if !self.client.is_open() {
            return Err(Error::NotConnected);
        }
        
        let result = Ok(self.sender_tx.send(DispatchMessage::Post(message)).await?);
        workflow_core::task::yield_now().await;
        result.map(|_| self)
    }

    pub async fn send(&self, message: Message) -> std::result::Result<&Self,Arc<Error>> {
        if !self.client.is_open() {
            return Err(Arc::new(Error::NotConnected));
        }
        
        let (sender, receiver) = oneshot(); 
        self.sender_tx.send(DispatchMessage::WithAck(message, sender)).await
            .map_err(|err|Arc::new(err.into()))?;
    
        receiver.recv().await.map_err(|_|Arc::new(Error::DispatchChannelAck))?
            .map(|_|self)
    }

    pub async fn recv(&self) -> Result<Message> {
        Ok(self.receiver_rx.recv().await?)
    }
}
