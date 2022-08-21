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

struct Inner {
    client : Arc<WebSocketInterface>,
    sender_tx : Sender<DispatchMessage>,
    receiver_rx : Receiver<Message>,
}

impl Inner {
    pub fn new(
        client: Arc<WebSocketInterface>,
        sender_tx: Sender<DispatchMessage>,
        receiver_rx: Receiver<Message>,
    ) -> Self {
        Self {
            client,
            sender_tx,
            receiver_rx,
        }
    }
}

#[derive(Clone)] 
pub struct WebSocket {
    inner: Arc<Inner>,
}

impl WebSocket {
    pub fn new(url : &str) -> Result<WebSocket> {

        let (receiver_tx, receiver_rx) = unbounded::<Message>();
        let (sender_tx, sender_tx_rx) = { 
            let tx_rx = unbounded::<DispatchMessage>();
            let tx = tx_rx.0.clone();
            (tx, tx_rx)
        };

        let client = Arc::new(WebSocketInterface::new(
            url,
            receiver_tx,
            sender_tx_rx
        )?);

        let websocket = WebSocket { 
            inner : Arc::new(Inner::new(
                client, sender_tx, receiver_rx
            )) 
        };

        Ok(websocket)
    }

    ///
    /// Returns reference to the transmission channel.
    /// You can clone the channel and use it directly.
    /// The channel sends DispatchMessage struct that
    /// has two values:
    /// 
    ///     - DispatchMessage::Post(Message)
    ///     - DispatchMessage::WithAck(Message, Sender<Result<Arc<(),Arc<Error>>>)
    /// 
    /// To use WithAck message, you need to create an instance
    /// of `oneshot` channel and supply the sender, while retain
    /// and await on receiver.recv() to get acknowledgement that
    /// the message has been successfully handed off to the underlying
    /// websocket.
    /// 
    pub fn sender_tx<'ws>(&'ws self) -> &'ws Sender<DispatchMessage> {
        &self.inner.sender_tx
    }

    /// Returns the reference to the receiver channel
    pub fn receiver_rx<'ws>(&'ws self) -> &'ws Receiver<Message> {
        &self.inner.receiver_rx
    }

    /// Connects the websocket to the destination URL.
    /// Optionally accepts `block_until_connected` argument
    /// that will block the async execution until the websocket
    /// is connected.
    pub async fn connect(&self, block_until_connected : bool) -> Result<()> {
        Ok(self.inner.client.connect(block_until_connected).await?)
    }

    /// Disconnects the websocket from the destination server.
    pub async fn disconnect(&self) -> Result<()> {
        Ok(self.inner.client.disconnect().await?)
    }

    /// Sends a message to the destination server. This function
    /// will queue the message on the relay channel and return
    /// successfully if the message has been queued.
    /// This function enforces async yield in order to prevent
    /// potential blockage of the executor if it is being executed
    /// in tight loops.
    pub async fn post(&self, message: Message) -> Result<&Self> {
        if !self.inner.client.is_open() {
            return Err(Error::NotConnected);
        }
        
        let result = Ok(self.inner.sender_tx.send(DispatchMessage::Post(message)).await?);
        workflow_core::task::yield_now().await;
        result.map(|_| self)
    }

    /// Sends a message to the destination server. This function
    /// will block until until the message was relayed to the
    /// underlying websocket implementation.
    pub async fn send(&self, message: Message) -> std::result::Result<&Self,Arc<Error>> {
        if !self.inner.client.is_open() {
            return Err(Arc::new(Error::NotConnected));
        }
        
        let (sender, receiver) = oneshot(); 
        self.inner.sender_tx.send(DispatchMessage::WithAck(message, sender)).await
            .map_err(|err|Arc::new(err.into()))?;
    
        receiver.recv().await.map_err(|_|Arc::new(Error::DispatchChannelAck))?
            .map(|_|self)
    }

    /// Receives message from the websocket. Blocks until a message is
    /// received from the underlying websocket connection.
    pub async fn recv(&self) -> Result<Message> {
        Ok(self.inner.receiver_rx.recv().await?)
    }
}
