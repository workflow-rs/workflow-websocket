
#[derive(Clone)]
pub enum DispatchStrategy {
    // None,
    Post,
    Ack,
}

pub struct Settings {
    pub strategy : DispatchStrategy
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            strategy: DispatchStrategy::Post
        }
    }
}