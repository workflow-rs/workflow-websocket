#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum State {
	Connecting,
	Open,
	Closing,
	Closed,
}
