pub struct Settings {
    // placeholder for future settings
    // TODO review if it makes sense to impl `reconnect_interval`
    pub receiver_channel_bounds : usize,
    pub sender_channel_bounds : usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            receiver_channel_bounds : 0,
            sender_channel_bounds : 0,
        }
    }
}

            