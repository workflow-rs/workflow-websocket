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

pub mod settings;
pub mod result;
pub mod error;
pub mod message;
pub mod event;
pub mod state;

pub use result::Result;
pub use error::Error;
pub use settings::Settings;
pub use message::*;

pub use message::Message;

use std::sync::Arc;
use async_std::channel::{Receiver,Sender,unbounded,bounded};
use workflow_core::channel::oneshot;
use workflow_core::trigger::Listener;

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
    pub fn new(url : &str, settings : Settings) -> Result<WebSocket> {

        let (receiver_tx, receiver_rx) = 
            if settings.receiver_channel_bounds == 0 {
                unbounded::<Message>()
            } else {
                bounded(settings.receiver_channel_bounds)
            };
        let (sender_tx, sender_tx_rx) = { 
            if settings.sender_channel_bounds == 0 {
                let tx_rx = unbounded::<DispatchMessage>();
                let tx = tx_rx.0.clone();
                (tx, tx_rx)
            } else {
                let tx_rx = bounded::<DispatchMessage>(settings.sender_channel_bounds);
                let tx = tx_rx.0.clone();
                (tx, tx_rx)
            }
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

    /// Get current websocket connection URL
    pub fn url(&self) -> String {
        self.inner.client.url()
    }

    /// Changes WebSocket connection URL.
    /// Following this call, you must invoke
    /// `WebSocket::reconnect().await` manually
    pub fn set_url(&self, url : &str) {
        self.inner.client.set_url(url);
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

    /// Returns true if websocket is connected, false otherwise
    pub fn is_open(&self) -> bool {
        self.inner.client.is_open()
    }

    /// Connects the websocket to the destination URL.
    /// Optionally accepts `block_until_connected` argument
    /// that will block the async execution until the websocket
    /// is connected.
    /// 
    /// Once invoked, connection task will run in the background
    /// and will attempt to repeatedly reconnect if the websocket
    /// connection is closed.
    /// 
    /// To suspend reconnection, you have to call `disconnect()`
    /// method explicitly.
    /// 
    pub async fn connect(&self, block_until_connected : bool) -> Result<Option<Listener>> {
        Ok(self.inner.client.connect(block_until_connected).await?)
    }

    /// Disconnects the websocket from the destination server.
    pub async fn disconnect(&self) -> Result<()> {
        Ok(self.inner.client.disconnect().await?)
    }

    /// Trigger WebSocket to reconnect.  This method
    /// closes the underlying WebSocket connection
    /// causing the WebSocket implementation to 
    /// re-initiate connection.
    pub async fn reconnect(&self) -> Result<()> {
        Ok(self.inner.client.close().await?)
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

    /// Helper function that will relay a Ctl enum to the receiver
    /// in the form of `Message::Ctl(Ctl::*)`
    /// This should be called only with Ctl::Custom(u32) as other
    /// control messages are issues by the underlying websocket implementation
    pub fn inject_ctl(&self, ctl : Ctl) -> Result<()> {
        Ok(self.inner.client.inject_ctl(ctl)?)
    }
}
